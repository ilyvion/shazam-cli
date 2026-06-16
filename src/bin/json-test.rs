fn main() {
    for path in std::env::args().skip(1) {
        let json = std::fs::read_to_string(&path).expect("failed to read file");
        println!("=== {path} ===");
        match shazam_lib::format_shazam_response(&json, None) {
            Ok(output) => println!("{output}"),
            Err(e) => println!("Error: {e}"),
        }
        println!();
    }
}
