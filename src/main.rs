use std::time::Duration;

fn main() {
    let args = sq::sys::Args {
        file: Some(cfg_select! {
            windows => "C:/Users/tomas/Documents/bonelesswing.mp3".to_string(),
            unix => "~/Documents/bonelesswing.mp3".to_string(),
        }),
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
    args.fade_to(0.5, Duration::from_secs(1));
    std::thread::sleep(Duration::from_secs(1));
    args.stop();
    std::thread::sleep(Duration::from_secs(1));
}
