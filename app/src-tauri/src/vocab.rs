//! Signed aliases for existing parser commands. Only grammar misses reach this
//! module, and every expanded command goes back through the built-in parser.
use crate::{
    model::Intent,
    parse::{self, ParseFailure, phrase},
    settings::{
        atomic_write,
        diagnostics::{self, Code, Event, Stage},
    },
};
use minisign_verify::{PublicKey, Signature};
use regex::{Regex, RegexBuilder};
use regex_syntax::ast::{Ast, GroupKind, parse::Parser};
use semver::Version;
use serde::Deserialize;
use std::{
    collections::HashSet,
    fs::{self, File, OpenOptions},
    io::{self, Read},
    path::Path,
    sync::{Arc, LazyLock, RwLock},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const BUNDLED: &[u8] = include_bytes!("../../../vocab/vocab.json");
const BUNDLED_SIGNATURE: &[u8] = include_bytes!("../../../vocab/vocab.json.minisig");
const PUBLIC_KEY: &str = include_str!("../../../vocab/vocab.pub");
const RELEASE: &str = "https://github.com/ruck-syu/Voxi/releases/download/vocab-latest";
const MAX_PACK_BYTES: usize = 64 * 1024;
const MAX_SIGNATURE_BYTES: usize = 1024;
const MAX_RULES: usize = 64;
const MAX_PATTERN_BYTES: usize = 512;
const MAX_EXAMPLES: usize = 8;
const MAX_TEMPLATE_BYTES: usize = 256;
const REGEX_SIZE_LIMIT: usize = 64 * 1024;
const CACHE: &str = "vocab.pack";
const CHECK_INTERVAL: Duration = Duration::from_secs(86_400);

// Only parser keywords can be literal template parts. Names, paths, extensions
// and numbers must be copied from a whole capture, without concatenation.
const KEYWORDS: &str = "a add all an and as by called combine convert copy count create date duplicates \
    every extract files find folder folders for from images in into list lowercase make merge month move \
    name named number of organize out pages pdf prefix rename replace resize show size sort \
    split the to type unzip with zip";

static APP_VERSION: LazyLock<Version> = LazyLock::new(|| Version::parse(env!("CARGO_PKG_VERSION")).expect("Cargo package version"));
static ACTIVE: LazyLock<RwLock<Arc<Pack>>> = LazyLock::new(|| {
    #[cfg(test)]
    let pack = bundled();
    #[cfg(not(test))]
    let pack = Pack::default();
    RwLock::new(Arc::new(pack))
});

#[path = "vocab/validate.rs"]
mod validate;
use validate::{IntentKind, checked_output};

fn bundled() -> Pack {
    verified_pack(BUNDLED, BUNDLED_SIGNATURE).and_then(|pack| pack.compile(&APP_VERSION, 0)).unwrap_or_default()
}

#[derive(Debug, PartialEq, Eq)]
enum Invalid {
    Size,
    Signature,
    Json,
    Schema,
    AppVersion,
    Rollback,
    Rule(String),
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PackFile {
    schema: u32,
    version: u64,
    min_app_version: Version,
    rules: Vec<RuleFile>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RuleFile {
    id: String,
    intent: IntentKind,
    pattern: String,
    replace: String,
    examples: Vec<Example>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Example {
    input: String,
    expect: String,
}

#[derive(Default)]
struct Pack {
    version: u64,
    rules: Vec<Rule>,
}

struct Rule {
    intent: IntentKind,
    pattern: Regex,
    template: Vec<Part>,
}

enum Part {
    Literal(String),
    Capture(usize),
}

fn verified_pack(bytes: &[u8], signature: &[u8]) -> Result<PackFile, Invalid> {
    if bytes.len() > MAX_PACK_BYTES || signature.len() > MAX_SIGNATURE_BYTES {
        return Err(Invalid::Size);
    }
    // The cache uses NUL as its separator. Minisign does not authenticate its
    // untrusted comment, so reject NUL anywhere in the detached signature.
    if signature.contains(&0) {
        return Err(Invalid::Signature);
    }
    let key = PublicKey::decode(PUBLIC_KEY).map_err(|_| Invalid::Signature)?;
    let signature = std::str::from_utf8(signature).map_err(|_| Invalid::Signature)?;
    let signature = Signature::decode(signature).map_err(|_| Invalid::Signature)?;
    key.verify(bytes, &signature, false).map_err(|_| Invalid::Signature)?;
    let pack = PackFile::decode(bytes)?;
    pack.check_comment(signature.trusted_comment())?;
    Ok(pack)
}

impl PackFile {
    fn decode(bytes: &[u8]) -> Result<Self, Invalid> {
        if bytes.len() > MAX_PACK_BYTES {
            return Err(Invalid::Size);
        }
        let pack: Self = serde_json::from_slice(bytes).map_err(|_| Invalid::Json)?;
        if pack.schema != 1 {
            return Err(Invalid::Schema);
        }
        Ok(pack)
    }

    fn check_comment(&self, comment: &str) -> Result<(), Invalid> {
        if comment != format!("Voxi vocabulary schema={} version={}", self.schema, self.version) {
            return Err(Invalid::Signature);
        }
        Ok(())
    }

    fn compile(self, app: &Version, floor: u64) -> Result<Pack, Invalid> {
        if !self.min_app_version.pre.is_empty()
            || !self.min_app_version.build.is_empty()
            || (*app != Version::new(0, 0, 0) && self.min_app_version > *app)
        {
            return Err(Invalid::AppVersion);
        }
        if self.version <= floor {
            return Err(Invalid::Rollback);
        }
        if self.rules.len() > MAX_RULES {
            return Err(Invalid::Size);
        }
        let mut ids = HashSet::new();
        let mut rules = Vec::new();
        for file in self.rules {
            if file.id.is_empty()
                || file.id.len() > 64
                || !file.id.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
                || !ids.insert(file.id.clone())
            {
                return Err(Invalid::Rule(file.id));
            }
            let rule = Rule::compile(&file).ok_or(Invalid::Rule(file.id))?;
            rules.push(rule);
        }
        Ok(Pack { version: self.version, rules })
    }
}

impl Rule {
    fn compile(file: &RuleFile) -> Option<Self> {
        if file.pattern.len() > MAX_PATTERN_BYTES
            || !file.pattern.starts_with('^')
            || !file.pattern.ends_with('$')
            || file.replace.len() > MAX_TEMPLATE_BYTES
            || !(1..=MAX_EXAMPLES).contains(&file.examples.len())
        {
            return None;
        }
        let body = &file.pattern[1..file.pattern.len() - 1];
        if !pattern_allowed(&Parser::new().parse(body).ok()?, false) {
            return None;
        }
        let pattern = RegexBuilder::new(&format!(r"\A(?:{body})\z"))
            .case_insensitive(true)
            .size_limit(REGEX_SIZE_LIMIT)
            .dfa_size_limit(REGEX_SIZE_LIMIT)
            .build()
            .ok()?;
        let mut next_capture = 1;
        let template = file
            .replace
            .split(' ')
            .map(|part| {
                if let Some(index) = part.strip_prefix('$') {
                    // Only the canonical "$1" form: no "$01", "$+1" or "${1}".
                    let index = index.parse::<usize>().ok().filter(|i| i.to_string() == index)?;
                    if index != next_capture || index >= pattern.captures_len() {
                        return None;
                    }
                    next_capture += 1;
                    Some(Part::Capture(index))
                } else {
                    KEYWORDS.split_ascii_whitespace().any(|keyword| keyword == part).then(|| Part::Literal(part.to_owned()))
                }
            })
            .collect::<Option<Vec<_>>>()?;
        if next_capture == 1 || next_capture != pattern.captures_len() {
            return None;
        }
        let rule = Self { intent: file.intent, pattern, template };
        for example in &file.examples {
            let baseline = parse::parse_builtin(&example.input);
            if matches!(baseline, Err(ParseFailure::HardRefusal(_))) {
                return None;
            }
            let input = phrase::normalize(&example.input);
            let (intent, output) = rule.rewrite(&input)?;
            if output != example.expect {
                return None;
            }
            if baseline.is_ok_and(|builtin| builtin != intent) {
                return None;
            }
        }
        Some(rule)
    }

    fn rewrite(&self, input: &str) -> Option<(Intent, String)> {
        let captures = self.pattern.captures(input)?;
        let quotes = phrase::quoted_ranges(input).ok()?;
        let mut output = String::new();
        for part in &self.template {
            let text = match part {
                Part::Literal(word) => word,
                Part::Capture(index) => {
                    let capture = captures.get(*index)?;
                    let range = capture.range();
                    // Whitespace boundaries also keep filenames, decimal values
                    // and paths indivisible. A quoted name is one argument.
                    if capture.as_str().is_empty()
                        || capture.as_str().trim() != capture.as_str()
                        || (range.start > 0 && !input[..range.start].ends_with(char::is_whitespace))
                        || (range.end < input.len() && !input[range.end..].starts_with(char::is_whitespace))
                        || quotes
                            .iter()
                            .any(|&(start, end)| (start < range.start && range.start < end) || (start < range.end && range.end < end))
                    {
                        return None;
                    }
                    capture.as_str()
                }
            };
            if output.len() + text.len() + usize::from(!output.is_empty()) > parse::MAX_COMMAND_BYTES {
                return None;
            }
            if !output.is_empty() {
                output.push(' ');
            }
            output.push_str(text);
        }
        Some((checked_output(self.intent, input, &output, &captures)?, output))
    }
}

/// Outside captures, only literal letters, whitespace, and anchors are allowed.
/// Alternatives and repetitions belong inside the text copied to the output.
fn pattern_allowed(ast: &Ast, in_capture: bool) -> bool {
    match ast {
        Ast::Group(group) => match group.kind {
            GroupKind::CaptureIndex(_) => !in_capture && pattern_allowed(&group.ast, true),
            GroupKind::NonCapturing(ref flags) => flags.items.is_empty() && pattern_allowed(&group.ast, in_capture),
            GroupKind::CaptureName { .. } => false,
        },
        Ast::Alternation(alt) => in_capture && alt.asts.iter().all(|ast| pattern_allowed(ast, true)),
        Ast::Concat(concat) => concat.asts.iter().all(|ast| pattern_allowed(ast, in_capture)),
        Ast::Repetition(rep) => in_capture && pattern_allowed(&rep.ast, true),
        Ast::Empty(_) => true,
        Ast::Literal(literal) => in_capture || (literal.c.is_alphabetic() && !literal.c.is_numeric()) || literal.c.is_whitespace(),
        // The outer ^ and $ are stripped before parsing, so any anchor left
        // outside a capture is a mid-pattern assertion.
        Ast::Assertion(_) => in_capture,
        node => in_capture && !matches!(node, Ast::Flags(_)),
    }
}

impl Pack {
    fn rewrite(&self, input: &str) -> Option<(Intent, String)> {
        self.rules.iter().find_map(|rule| rule.rewrite(input))
    }
}

fn active() -> Arc<Pack> {
    Arc::clone(&ACTIVE.read().unwrap_or_else(|error| error.into_inner()))
}

pub(crate) fn rewrite(input: &str) -> Option<(Intent, String)> {
    #[cfg(test)]
    if let Some(result) = TEST_PACK.with_borrow(|pack| pack.as_ref().map(|pack| pack.rewrite(input))) {
        return result;
    }
    active().rewrite(input)
}

#[cfg(test)]
thread_local! {
    static TEST_PACK: std::cell::RefCell<Option<Pack>> = const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
pub(crate) use tests::with_test_pack;

fn activate(pack: Pack) {
    let mut active = ACTIVE.write().unwrap_or_else(|error| error.into_inner());
    if pack.version > active.version {
        *active = Arc::new(pack);
        report_version(active.version);
    }
}

fn report_version(version: u64) {
    diagnostics::emit(Event::new(Code::Vocabulary, Stage::Active).vocab_version(version));
}

fn read_limited(path: &Path, limit: usize) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    File::open(path)?.take(limit as u64 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > limit {
        return Err(io::Error::from(io::ErrorKind::InvalidData));
    }
    Ok(bytes)
}

// One atomic cache entry keeps the detached signature paired with its payload.
fn load_cache(dir: &Path, floor: u64) -> (Option<Pack>, u64) {
    let Ok(cache) = read_limited(&dir.join(CACHE), MAX_PACK_BYTES + MAX_SIGNATURE_BYTES + 1) else { return (None, floor) };
    let Some(separator) = cache.iter().position(|byte| *byte == 0) else { return (None, floor) };
    let Ok(pack) = verified_pack(&cache[separator + 1..], &cache[..separator]) else { return (None, floor) };
    let version = pack.version;
    match pack.compile(&APP_VERSION, floor) {
        Ok(pack) => (Some(pack), version),
        // An older app may need its fallback, but it must not overwrite a pack
        // saved by a newer installation with a lower version.
        Err(Invalid::AppVersion) => (None, floor.max(version)),
        Err(_) => (None, floor),
    }
}

fn initialize(dir: &Path) -> u64 {
    let bundled = bundled();
    let (cached, floor) = load_cache(dir, bundled.version);
    let pack = cached.unwrap_or(bundled);
    report_version(pack.version);
    *ACTIVE.write().unwrap_or_else(|error| error.into_inner()) = Arc::new(pack);
    floor
}

async fn download(client: &reqwest::Client, asset: &str, limit: usize) -> Option<Vec<u8>> {
    let mut response = client.get(format!("{RELEASE}/{asset}")).send().await.ok()?.error_for_status().ok()?;
    if response.content_length().is_some_and(|len| len > limit as u64) {
        return None;
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.ok()? {
        if chunk.len() > limit - body.len() {
            return None;
        }
        body.extend_from_slice(&chunk);
    }
    Some(body)
}

fn seconds_since_epoch() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs()
}

fn check_due(dir: &Path, now: u64) -> Duration {
    let last = read_limited(&dir.join("vocab.last-check"), 20).ok().and_then(|bytes| std::str::from_utf8(&bytes).ok()?.parse::<u64>().ok());
    last.map_or(Duration::ZERO, |last| CHECK_INTERVAL.saturating_sub(Duration::from_secs(now.saturating_sub(last))))
}

fn refresh(dir: &Path, first: bool) -> Duration {
    let lock = (|| {
        fs::create_dir_all(dir)?;
        OpenOptions::new().create(true).truncate(false).write(true).open(dir.join("vocab.lock"))
    })();
    let Ok(lock) = lock else {
        if first {
            initialize(dir);
        }
        return CHECK_INTERVAL;
    };
    // Hold the lock through caching to serialize separate instances, and record
    // the attempt before making a request even if the network is offline.
    if lock.try_lock().is_err() {
        if first {
            initialize(dir);
        }
        return Duration::from_secs(60);
    }
    let initial_floor = first.then(|| initialize(dir));
    let now = seconds_since_epoch();
    let delay = check_due(dir, now);
    if !delay.is_zero() {
        return delay;
    }
    if atomic_write(&dir.join("vocab.last-check"), now.to_string().as_bytes()).and_then(|write| write.require_durable()).is_err() {
        return CHECK_INTERVAL;
    }
    let floor = initial_floor.unwrap_or_else(|| {
        let (cached, floor) = load_cache(dir, active().version);
        if let Some(pack) = cached {
            activate(pack);
        }
        floor
    });
    tauri::async_runtime::block_on(refresh_pack(dir, floor));
    check_due(dir, seconds_since_epoch()).max(Duration::from_secs(1))
}

async fn refresh_pack(dir: &Path, floor: u64) {
    // Separate, cookie-less client with no app metadata or command data.
    let Ok(client) = client() else { return };
    let Some(bytes) = download(&client, "vocab.json", MAX_PACK_BYTES).await else { return };
    let Some(signature) = download(&client, "vocab.json.minisig", MAX_SIGNATURE_BYTES).await else { return };
    let Ok(pack) = verified_pack(&bytes, &signature).and_then(|pack| pack.compile(&APP_VERSION, floor)) else { return };
    let mut cache = signature;
    cache.push(0);
    cache.extend_from_slice(&bytes);
    if atomic_write(&dir.join(CACHE), &cache).and_then(|write| write.require_durable()).is_ok() {
        activate(pack);
    }
}

fn client() -> reqwest::Result<reqwest::Client> {
    // The updater uses the same ring provider, but might not have run yet.
    let _ = rustls::crypto::ring::default_provider().install_default();
    reqwest::Client::builder().https_only(true).timeout(Duration::from_secs(5)).build()
}

pub(crate) fn start(dir: &Path) {
    let dir = dir.to_owned();
    let _ = std::thread::Builder::new().name("voxi-vocab".into()).spawn(move || {
        std::thread::sleep(refresh(&dir, true));
        loop {
            std::thread::sleep(refresh(&dir, false));
        }
    });
}

#[cfg(test)]
#[path = "vocab/tests.rs"]
mod tests;
