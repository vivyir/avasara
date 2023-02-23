use avasara::{
    analyze_pitch, decode, interleave_to_mono,
    vorbis_rs::{VorbisBitrateManagementStrategy, VorbisEncoder},
};
#[allow(unused_imports)]
use avasara::{OggToOgg, Remuxer};
use std::{
    io::{Cursor, Read, Write},
    num::{NonZeroU32, NonZeroU8},
};

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

    println!("decoding {}...", path);
    let (audio, sample_rate, channels) = decode(src);
    if (sample_rate == 0) || (channels == 0) {
        eprintln!("there was an error trying to decode the metadata of the source, your audio file is most likely broken, exiting... (can't continue without knowing channel count and sample rate)");
        std::process::exit(-1);
    }
    println!(
        "decoded! sample rate: {}, channel count: {}",
        sample_rate, channels
    );

    println!("doing pitch analysis...");
    // 50Hz to 600Hz only, prioritizing the human vocal range
    let (pitch_report, _pitch_points) = analyze_pitch(&audio, sample_rate, 50.0, 600.0);
    println!(
        "pitch analysis done! average: {}, median: {}, lowest: {}, highest: {}",
        pitch_report.mean, pitch_report.median, pitch_report.lowest, pitch_report.highest,
    );

    println!("interleaving to mono...");
    let mut audio = interleave_to_mono(audio, sample_rate as u32, channels);
    println!("interleaved!");

    println!("encoding to ogg...");
    // 24000 because we resampled to 24kHz, and 1 because 1 channel, mono, whatever
    let mut ogg_out = vec![];

    let mut encoder = VorbisEncoder::new(
        0,                                            // i picked 0 randomly i promise
        [("", ""); 0],                                // no comments
        NonZeroU32::new(sample_rate as u32).unwrap(), // target sample rate
        NonZeroU8::new(1).unwrap(),                   // 1 channel, because we interleaved to mono
        // very low quality, but not the absolute lowest.
        VorbisBitrateManagementStrategy::QualityVbr {
            target_quality: -0.15,
        },
        None,
        &mut ogg_out,
    )
    .unwrap();

    // chunks of 512 satisfied the vorbis gods, so i'll use it here
    for i in audio.as_f32_slice().chunks(512) {
        encoder.encode_audio_block(&[i]).unwrap();
    }
    encoder.finish().unwrap();
    println!("encoding finished!");

    /* this code exists as a reference but it can break duration headers, not worth saving 30kb
     * but you may want it.
    println!("remuxing and optimizing...");

    let mut remuxed = vec![];
    OggToOgg::new_with_defaults().remux(&mut Cursor::new(ogg_out), &mut remuxed).unwrap();

    println!("the optimized file is {} bytes and was saved to {}.", remuxed.len(), format!("{}.ogg", path));
    */

    println!(
        "the encoded file is {} bytes and was saved to {}.",
        ogg_out.len(),
        format!("{}.ogg", path)
    );

    let mut output = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .open(format!("{}.ogg", path))
        .unwrap();
    output.write_all(&ogg_out).unwrap();
}
