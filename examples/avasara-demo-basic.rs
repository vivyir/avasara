use avasara::compose_to_ogg;
use std::io::{Cursor, Read, Write};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let path = args.get(1).expect("file path not provided");

    // or instead of all of this you could just get a Vec<u8> and wrap it in a cursor
    let mut src = vec![];
    std::fs::File::open(&path)
        .expect("failed to open media")
        .read_to_end(&mut src)
        .unwrap();
    let src = Cursor::new(src);

    let opus = compose_to_ogg(src, path, 0, -0.2, true);
    println!(
        "The encoded file is {} bytes and was saved to {}.",
        opus.len(),
        format!("{}.ogg", path)
    );
    let mut output = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .open(format!("{}.ogg", path))
        .unwrap();
    output.write_all(&opus).unwrap();
}
