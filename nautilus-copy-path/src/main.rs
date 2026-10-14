use copypath::copy_and_notify_async;
use std::env;
use std::process;
use std::thread;
use std::time::Duration;

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.is_empty() {
        eprintln!("Usage: copy-path <file-or-dir-path> [...]");
        process::exit(1);
    }

    let combined = args.join("\n");
    copy_and_notify_async(combined);

    // Give background thread a short moment before CLI exits
    thread::sleep(Duration::from_millis(50));
}
