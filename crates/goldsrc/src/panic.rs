//! Panic hook: report through the engine console and a log file before `panic = "abort"`
//! kills the process. A panic must never unwind across an `extern "C"` boundary.

use std::io::Write;

pub fn install(log_file: &'static str, print: fn(&str)) {
    std::panic::set_hook(Box::new(move |info| {
        let bt = std::backtrace::Backtrace::force_capture();
        let msg = format!("PANIC: {info}\n");
        print(&msg);
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(log_file) {
            let _ = writeln!(f, "{msg}{bt}");
        }
    }));
}
