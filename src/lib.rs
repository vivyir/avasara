pub use fon::chan::Ch32;
pub use fon::Audio;
pub use optivorbis::{OggToOgg, Remuxer};
pub use vorbis_rs;

use std::io::Cursor;
use std::num::{NonZeroU32, NonZeroU8};

use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{DecoderOptions, CODEC_TYPE_NULL};
use symphonia::core::errors::Error;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

use pitch_detection::detector::{yin::YINDetector, PitchDetector};
use vorbis_rs::{VorbisBitrateManagementStrategy, VorbisEncoder};

/// Decodes from many formats (namely: aac, alac, flac, isomp4, mkv, mp3, ogg, pcm, vorbis and of
/// course, wav) using a cursor so that it can support both in-memory and on-disk audio, outputs
/// a vector of f32 sound samples, sample rate, and channel count respectively.
pub fn decode(src: Cursor<Vec<u8>>) -> (Vec<f32>, usize, usize) {
    // more info at getting_started.md of Symphonia
    let mss = MediaSourceStream::new(Box::new(src), Default::default());
    let hint = Hint::new();
    let meta_opts: MetadataOptions = Default::default();
    let fmt_opts: FormatOptions = Default::default();

    let probed = symphonia::default::get_probe()
        .format(&hint, mss, &fmt_opts, &meta_opts)
        .expect("unsupported format");

    let mut format = probed.format;

    let track = format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
        .expect("no supported audio tracks");

    let dec_opts: DecoderOptions = Default::default();

    // will later be set in the decode loop using spec
    let mut sample_rate = 0;
    let mut channels = 0;

    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &dec_opts)
        .expect("unsupported codec");

    let track_id = track.id;

    let mut audio = vec![];

    loop {
        let packet = match format.next_packet() {
            Ok(packet) => packet,
            Err(Error::ResetRequired) => {
                unimplemented!(); // NOTE: refer to getting_started.md of Symphonia
            }
            Err(err) => {
                if (&err).to_string() == "end of stream" {
                    // this is called whenever audiofile finishes reading
                    break;
                } else {
                    // an unrecoverable error occured, halt decoding.
                    // FIXME: return error using Result
                    panic!("{}", err);
                }
            }
        };

        while !format.metadata().is_latest() {
            format.metadata().pop();

            /*
            if let Some(rev) = format.metadata().current() {
                dbg!(&rev);
            }
            */

            // consume the new metadata at the head of the metadata queue...? dont think i need it
        }

        if packet.track_id() != track_id {
            continue;
        }

        match decoder.decode(&packet) {
            Ok(decoded) => {
                let spec = decoded.spec().clone().to_owned();
                channels = spec.channels.count();
                sample_rate = spec.rate;

                let mut sample_buf =
                    SampleBuffer::<f32>::new(decoded.capacity() as u64, *decoded.spec());
                sample_buf.copy_interleaved_ref(decoded);
                let samples = sample_buf.samples();
                for i in samples {
                    audio.push(*i);
                }
            }
            Err(Error::IoError(_)) => {
                continue;
            }
            Err(Error::DecodeError(_)) => {
                continue;
            }
            Err(err) => {
                // an unrecoverable error occured, halt decoding.
                // FIXME: return err using Result
                panic!("{}", err);
            }
        }
    }

    (audio, sample_rate as usize, channels)
}

fn mean(list: &[f32]) -> f32 {
    list.iter().sum::<f32>() / list.len() as f32
}

fn median(list: &[f32]) -> f32 {
    let len = list.len();
    let mid = len / 2;
    if len % 2 == 0 {
        mean(&list[(mid - 1)..(mid + 1)])
    } else {
        list[mid]
    }
}

/// Basic report of the pitch values observed, returned by `analyze_pitch`, refer to it's
/// documentation for more in-depth explanation.
#[derive(Debug, Clone, Copy)]
pub struct PitchReport {
    /// This is the percentage of chunks (1024 sound samples) used out of the whole audio data,
    /// this is out of 100, so 17.7[...] would signify 17.7[...]% percent of the chunks were used
    /// as valid pitch points, and the rest were discarded because they were either way too high or
    /// way too low, or just couldn't be measured with YIN at all, rule of thumb for instrumentals
    /// is that this should be higher than 1 percent (with a min frequency of 50Hz and max
    /// frequency of 600Hz) to be credible, and for human speech (with the same min/max
    /// frequencies) it should be above 10 percent, of course you can also just ignore this
    /// altogether.
    pub chunks_used: f64,
    pub mean: f32,
    pub median: f32,
    pub lowest: f32,
    pub highest: f32,
}

/// Analyze the pitch of some audio data, it splits up the audio data into chunks of 1024 sound
/// samples, then it uses the YIN algorithm from the "pitch-detection" crate to do the highest
/// quality possible pitch analysis on those chunks, some chunks however aren't suitable for
/// analysis which automatically get discarded, out of the chunks left it'll clamp them using the
/// `min_frequency` and `max_frequency` parameters, then from the clamped pitch points because they
/// may still contain some outliers with extremely high/extremely low frequencies we sort them,
/// take the highest 10% and discard it, same with the lowest 10%, which leaves us with only 80% of
/// the clamped pitch points, this usually means that we'll usually be getting very accurate and
/// very clean pitch points; this function will return those as a Vec<f32> (each value is in Hz)
/// along with a basic `PitchReport`, containing mean, median, lowest, and highest pitch points
/// observed for convenience. (Along with a `chunks_used` which is more complex, refer to
/// `PitchReport`'s documentation)
///
/// Note: This function will panic if there aren't any valid pitch points (within your min/max
/// frequencies) in your audio data.
pub fn analyze_pitch(
    audio_data: &[f32],
    sample_rate: usize,
    min_frequency: f32,
    max_frequency: f32,
) -> (PitchReport, Vec<f32>) {
    let mut pitch_points: Vec<(f32, f32)> = vec![];
    for chunk in audio_data.chunks(1024) {
        let mut detector = YINDetector::new(chunk.len(), chunk.len() / 2);
        // power threshold and clarity threshold are zero, we'll clamp the array ourselves instead.
        let pitch_opt = detector.get_pitch(chunk, sample_rate, 0.0, 0.0);
        if let Some(v) = pitch_opt {
            pitch_points.push((v.frequency, v.clarity))
        }
    }

    // TODO: convert into an iterator, it'll be faster
    let mut freqvec = vec![];
    for (freq, _) in &pitch_points {
        if (*freq > min_frequency as f32) && (*freq < max_frequency as f32) {
            freqvec.push(*freq);
        }
    }

    // sorts the pitch points smallest to highest, then takes the bottom 10% (low) and high
    // 10% (high), then removes both of them leaving 80% of the values, practically removing
    // the extreme highs and extreme lows.
    freqvec.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let low = (freqvec.len() as f64 * 0.10).round() as usize;
    let high = freqvec.len() - low;
    let freqvec: Vec<f32> = freqvec[low..high].to_vec();

    // what the actual fuck is happening
    let pitch_report = PitchReport {
        chunks_used: (freqvec.len() as f64 / (audio_data.len() as f64 / 1024.0)) * 100.0,
        mean: mean(&freqvec),
        median: median(&freqvec),
        lowest: *freqvec.first().unwrap(),
        highest: *freqvec.last().unwrap(),
    };

    (pitch_report, freqvec)
}

/// Interleaves a vector of f32 sound samples to make mono if stereo, doesn't support more
/// channels, returns an `Audio<Ch32, 1>` (mono pcm data)
pub fn interleave_to_mono(
    audio: Vec<f32>,
    sample_rate: u32,
    src_channels: usize,
) -> Audio<Ch32, 1> {
    if src_channels == 1 {
        let audio = Audio::<Ch32, 1>::with_f32_buffer(sample_rate, audio);

        audio
    } else if src_channels == 2 {
        let audio = Audio::<Ch32, 2>::with_f32_buffer(sample_rate, audio);
        let audio = Audio::<Ch32, 1>::with_audio(sample_rate, &audio);

        audio
    } else if src_channels > 2 {
        panic!("more than 2 channels provided");
    } else {
        panic!("No channels? (megamind stare)");
    }
}

/// Convenience function which chains the decoding step, the interleaving step and the encoding
/// step together, for `src` you can load a file as bytes and convert it into a Cursor<Vec<u8>>, or
/// use in-memory audio data wrapped in a cursor and use it, for a list of the formats it can
/// decode refer to `decode`'s documentation, then it interleaves it to mono and then encodes it
/// into Ogg Vorbis, using the `stream_serial`, `target_quality` and `remux` arguments for that,
/// `stream_serial` really doesn't matter, just pick a random 32bit integer or use 0 or something,
/// `target_quality` is a float between `-0.2` and `2.0`, the lowest one meaning more compression
/// and less quality and the bigger one vice versa, `remux` is just for whether you want to use
/// `optivorbis` to do a two-pass optimization on the result, may or may not be useful but it
/// exists ig. Feel free to look at the source for a reference of how you can make a function like
/// this using Avasara yourself!
///
/// # Panics
/// Panics if there was a problem while decoding, i highly recommend you write your own function by
/// looking at the source of this one, this is not at all intended for production use, only as a
/// convenience function for prototyping.
pub fn compose_to_ogg(
    src: Cursor<Vec<u8>>,
    prefix: &str,
    stream_serial: i32,
    target_quality: f32,
    remux: bool,
) -> Vec<u8> {
    println!("[{}] Decoding", prefix);
    let (audio, sample_rate, channels) = decode(src);
    if (sample_rate == 0) || (channels == 0) {
        // FIXME: why tf did i use panic!?
        panic!("Sample rate or channel count was zero, indicating that there was a problem with decoding, exiting.");
    }
    println!("[{}] Decoded", prefix);

    // more info at Fōn/fon's readme
    println!("[{}] Interleaving", prefix);
    let mut audio = interleave_to_mono(audio, sample_rate as u32, channels);
    println!("[{}] Interleaved", prefix);

    println!("[{}] Encoding (to Ogg Vorbis)", prefix);
    let mut ogg_out = vec![];

    let mut encoder = VorbisEncoder::new(
        stream_serial,
        [("", ""); 0],
        NonZeroU32::new(sample_rate as u32).unwrap(),
        NonZeroU8::new(1).unwrap(), // because mono
        VorbisBitrateManagementStrategy::QualityVbr {
            target_quality, // 2.0 to -0.2
        },
        None,
        &mut ogg_out,
    )
    .unwrap();

    // i just guessed chunking the massive f32 buffer into itty bitty 512 element chunks would work
    // and not segfault when trying to encode, and it did, pretty cool ig. FIXME: too many unwraps
    for i in audio.as_f32_slice().chunks(512) {
        encoder.encode_audio_block(&[i]).unwrap();
    }
    encoder.finish().unwrap();

    if remux {
        let mut out = vec![];

        OggToOgg::new_with_defaults()
            .remux(&mut Cursor::new(ogg_out), &mut out)
            .unwrap();
        println!("[{}] Encoded and remuxed!", prefix);

        out
    } else {
        println!("[{}] Encoded!", prefix);
        ogg_out
    }
}
