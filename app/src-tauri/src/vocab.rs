//! Signed aliases for existing parser commands. Only grammar misses reach this
//! module, and every expanded command goes back through the built-in parser.
use crate::{
    model::{Destination, Intent, Source},
    parse::{self, ParseFailure, phrase},
    settings::{
        atomic_write,
        diagnostics::{self, Code, Event, Stage},
    },
};
use minisign_verify::{PublicKey, Signature};
use regex::{Regex, RegexBuilder};
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
const KEYWORDS: &str = "a actions add all an and as by called combine convert copy count create date delete duplicates \
    every extract files find finder folder folders for from images in including into list lowercase make merge month move \
    name named number of open organize out pages pdf prefix recent remove rename replace resize reveal show size sort \
    split the to trash type undo unzip with zip";

static APP_VERSION: LazyLock<Version> = LazyLock::new(|| Version::parse(env!("CARGO_PKG_VERSION")).expect("Cargo package version"));
static ACTIVE: LazyLock<RwLock<Arc<Pack>>> = LazyLock::new(|| {
    let pack = verified_pack(BUNDLED, BUNDLED_SIGNATURE).and_then(|pack| pack.compile(&APP_VERSION, 0));
    RwLock::new(Arc::new(pack.unwrap_or_else(|_| Pack { version: 0, rules: Vec::new() })))
});

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

struct Pack {
    version: u64,
    rules: Vec<Rule>,
}

struct Rule {
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
    let key = PublicKey::decode(PUBLIC_KEY).map_err(|_| Invalid::Signature)?;
    let signature = std::str::from_utf8(signature).map_err(|_| Invalid::Signature)?;
    let signature = Signature::decode(signature).map_err(|_| Invalid::Signature)?;
    key.verify(bytes, &signature, false).map_err(|_| Invalid::Signature)?;
    PackFile::decode(bytes)
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

    fn compile(self, app: &Version, floor: u64) -> Result<Pack, Invalid> {
        if !self.min_app_version.pre.is_empty() || !self.min_app_version.build.is_empty() || self.min_app_version > *app {
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
        let pattern = RegexBuilder::new(&file.pattern)
            .case_insensitive(true)
            .size_limit(REGEX_SIZE_LIMIT)
            .dfa_size_limit(REGEX_SIZE_LIMIT)
            .build()
            .ok()?;
        let mut used = HashSet::new();
        let template = file
            .replace
            .split(' ')
            .map(|part| {
                if let Some(index) = part.strip_prefix('$') {
                    let index = index.parse::<usize>().ok()?;
                    (index > 0 && index < pattern.captures_len() && used.insert(index)).then_some(Part::Capture(index))
                } else {
                    KEYWORDS.split_ascii_whitespace().any(|keyword| keyword == part).then(|| Part::Literal(part.to_owned()))
                }
            })
            .collect::<Option<Vec<_>>>()?;
        let rule = Self { pattern, template };
        for example in &file.examples {
            let baseline = parse::parse_builtin(&example.input);
            if matches!(baseline, Err(ParseFailure::HardRefusal(_))) {
                return None;
            }
            let input = phrase::normalize(&example.input);
            let output = rule.expand(&input)?;
            if output != example.expect {
                return None;
            }
            let intent = checked_output(&input, &output)?;
            if baseline.is_ok_and(|builtin| builtin != intent) {
                return None;
            }
        }
        Some(rule)
    }

    fn expand(&self, input: &str) -> Option<String> {
        let captures = self.pattern.captures(input)?;
        // Anchors can bind to only one branch of an alternation. Require a
        // whole-command match independently of the pattern's anchor placement.
        if captures.get(0)?.range() != (0..input.len()) {
            return None;
        }
        let mut output = String::new();
        for part in &self.template {
            let text = match part {
                Part::Literal(word) => word,
                Part::Capture(index) => captures.get(*index)?.as_str(),
            };
            if output.len() + text.len() + usize::from(!output.is_empty()) > parse::MAX_COMMAND_BYTES {
                return None;
            }
            if !output.is_empty() {
                output.push(' ');
            }
            output.push_str(text);
        }
        Some(output)
    }
}

fn checked_output(input: &str, output: &str) -> Option<Intent> {
    let intent = parse::parse_vocab_output(output)?;
    // The built-in parser can also consume a keyword as a name in an argument
    // position. Check those values against the original normalized input.
    let input = input.to_lowercase();
    let tokens: HashSet<_> = name_tokens(&input).collect();
    let present = |name: &str| name_tokens(&name.to_lowercase()).all(|token| tokens.contains(token));
    let destination = |dest: &Destination| match dest {
        Destination::Path(path) => present(path),
        Destination::InFolder { folder, parent, literal, .. } => {
            present(folder) && present(parent) && literal.as_deref().is_none_or(present)
        }
    };
    let source_ok = match intent.source() {
        Some(Source::Item { name, folder }) => present(name) && folder.as_deref().is_none_or(present),
        Some(Source::Set { folder, filter }) => {
            [folder.as_deref(), filter.name_contains.as_deref(), filter.name_starts_with.as_deref()].into_iter().flatten().all(present)
                && filter.ext.iter().flatten().all(|ext| present(ext))
        }
        None => true,
    };
    let arguments_ok = match &intent {
        Intent::Move { to, .. }
        | Intent::Copy { to, .. }
        | Intent::SortByType { folder: to, .. }
        | Intent::SortByMonth { folder: to, .. } => destination(to),
        Intent::Rename { new_name: name, .. }
        | Intent::AddPrefix { prefix: name, .. }
        | Intent::Number { base: name, .. }
        | Intent::Size { folder: name }
        | Intent::Duplicates { folder: name }
        | Intent::Reveal { target: name } => present(name),
        Intent::ReplaceInNames { find, replace, .. } => present(find) && present(replace),
        Intent::CreateFolders { names, parent } => names.iter().all(|name| present(name)) && parent.as_deref().is_none_or(present),
        Intent::MergePdf { output, .. } => output == "merged.pdf" || present(output), // Built-in default.
        Intent::ExtractPages { output, .. }
        | Intent::ImagesToPdf { output, .. }
        | Intent::Zip { output, .. }
        | Intent::Unzip { destination: output, .. } => output.as_deref().is_none_or(present),
        _ => true,
    };
    (source_ok && arguments_ok).then_some(intent)
}

fn name_tokens(text: &str) -> impl Iterator<Item = &str> {
    static TOKEN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[\p{L}\p{M}\p{N}_]+(?:[./-][\p{L}\p{M}\p{N}_]+)*").unwrap());
    TOKEN.find_iter(text).map(|m| m.as_str())
}

impl Pack {
    fn rewrite(&self, input: &str) -> Option<Intent> {
        let output = self.rules.iter().find_map(|rule| rule.expand(input))?;
        checked_output(input, &output)
    }
}

fn active() -> Arc<Pack> {
    Arc::clone(&ACTIVE.read().unwrap_or_else(|error| error.into_inner()))
}

pub(crate) fn rewrite(input: &str) -> Option<Intent> {
    active().rewrite(input)
}

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
fn load_cache(dir: &Path) -> u64 {
    let floor = active().version;
    let Ok(cache) = read_limited(&dir.join(CACHE), MAX_PACK_BYTES + MAX_SIGNATURE_BYTES + 1) else { return floor };
    let Some(separator) = cache.iter().position(|byte| *byte == 0) else { return floor };
    let Ok(pack) = verified_pack(&cache[separator + 1..], &cache[..separator]) else { return floor };
    let version = pack.version;
    match pack.compile(&APP_VERSION, floor) {
        Ok(pack) => activate(pack),
        // An older app may need its fallback, but it must not overwrite a pack
        // saved by a newer installation with a lower version.
        Err(Invalid::AppVersion) => return floor.max(version),
        Err(_) => {}
    }
    active().version
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

fn refresh(dir: &Path) -> Duration {
    let lock = (|| {
        fs::create_dir_all(dir)?;
        OpenOptions::new().create(true).truncate(false).write(true).open(dir.join("vocab.lock"))
    })();
    let Ok(lock) = lock else { return CHECK_INTERVAL };
    // Hold the lock through caching to serialize separate instances, and record
    // the attempt before making a request even if the network is offline.
    if lock.try_lock().is_err() {
        return Duration::from_secs(60);
    }
    let floor = load_cache(dir);
    let now = seconds_since_epoch();
    let delay = check_due(dir, now);
    if !delay.is_zero() {
        return delay;
    }
    if atomic_write(&dir.join("vocab.last-check"), now.to_string().as_bytes()).and_then(|write| write.require_durable()).is_err() {
        return CHECK_INTERVAL;
    }
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
    load_cache(dir);
    report_version(active().version);
    let dir = dir.to_owned();
    let _ = std::thread::Builder::new().name("voxi-vocab".into()).spawn(move || {
        loop {
            std::thread::sleep(refresh(&dir));
        }
    });
}

#[cfg(test)]
#[path = "vocab/tests.rs"]
mod tests;
