use std::time::Duration;

fn main() {
    let mut args = sq::sys::Args {
        file: Some("/Users/tomasmiglio/Documents/bonelesswing.mp3".to_string()),
        ..Default::default()
    };
    for warn in sq::sys::warn_extreme_settings(&args) {
        println!("{warn}");
    }
    let thread_args = args.clone();
    std::thread::spawn(move || {
        if let Err(e) = sq::sys::run(&thread_args) {
            eprintln!("Error: {e:#}");
        }
    });
    std::thread::sleep(Duration::from_secs(5));
    args.fade_to(0.5, 1.0);
    std::thread::sleep(Duration::from_secs(1));
    args.stop();
    std::thread::sleep(Duration::from_secs(1));
}
