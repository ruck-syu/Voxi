use serde_json::{json, Value};
use std::{
    env,
    io::{self, BufRead, Write},
    path::PathBuf,
    time::Instant,
};
use voxi_tiny_native::NativeRewriter;

fn main() {
    if let Err(e) = run() {
        eprintln!("{e}");
        std::process::exit(2);
    }
}

fn run() -> Result<(), String> {
    let mut args = env::args().skip(1);
    let mut model_dir = None;
    let mut threads = 1usize;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--model-dir" => model_dir = args.next().map(PathBuf::from),
            "--threads" => {
                threads = args
                    .next()
                    .ok_or("missing --threads value")?
                    .parse()
                    .map_err(|_| "invalid --threads")?
            }
            "--help" => {
                println!("voxi-tiny-native --model-dir DIR [--threads N] < input.jsonl");
                return Ok(());
            }
            other => return Err(format!("unknown argument {other}")),
        }
    }
    if threads == 0 || threads > 64 {
        return Err("--threads must be 1..64".into());
    }
    let dir = model_dir.ok_or("--model-dir is required")?;
    let load_start = Instant::now();
    let mut rewriter = NativeRewriter::load_with_threads(&dir, threads)
        .map_err(|e| format!("load failed: {e}"))?;
    eprintln!(
        "model_load_ms={:.3}",
        load_start.elapsed().as_secs_f64() * 1000.0
    );
    let stdin = io::stdin();
    let mut stdout = io::BufWriter::new(io::stdout().lock());
    for line in stdin.lock().lines() {
        let line = line.map_err(|e| e.to_string())?;
        let start = Instant::now();
        let parsed: Result<Value, _> = serde_json::from_str(&line);
        let (id, text, initial_error) = match parsed {
            Ok(v) => {
                let id = v.get("id").cloned().unwrap_or(Value::Null);
                let id_ok = id.is_string() || id.is_number();
                let text = v.get("text").and_then(Value::as_str).map(str::to_owned);
                let error = if !id_ok {
                    Some("invalid_input: id must be a string or number".into())
                } else if text.is_none() {
                    Some("invalid_input: text must be a string".into())
                } else {
                    None
                };
                (id, text, error)
            }
            Err(e) => (Value::Null, None, Some(format!("invalid_json: {e}"))),
        };
        let (rewrite, error) = if let Some(e) = initial_error {
            (Value::Null, Some(e))
        } else if let Some(text) = text.as_deref() {
            match rewriter.rewrite(text) {
                Ok(s) => (Value::String(s), None),
                Err(e) => (Value::Null, Some(e.to_string())),
            }
        } else {
            (
                Value::Null,
                Some("invalid_input: text must be a string".into()),
            )
        };
        let mut result = json!({"id":id,"text":text,"rewrite":rewrite,"elapsed_ms":start.elapsed().as_secs_f64()*1000.0});
        if let Some(e) = error {
            result["error"] = Value::String(e);
        }
        serde_json::to_writer(&mut stdout, &result).map_err(|e| e.to_string())?;
        stdout.write_all(b"\n").map_err(|e| e.to_string())?;
    }
    stdout.flush().map_err(|e| e.to_string())?;
    Ok(())
}
