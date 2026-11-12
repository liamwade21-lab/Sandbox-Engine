use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

struct Rec {
    data: Vec<f32>,
    start: Instant,
    rate: u32,
    ch: usize,
}

fn push(rec: &Arc<Mutex<Rec>>, chunk: &[f32]) {
    let mut r = rec.lock().unwrap();
    let frames = chunk.len() / r.ch;
    let now_f = (r.start.elapsed().as_secs_f64() * r.rate as f64) as i64;
    let chunk_start = now_f - frames as i64;
    let have = (r.data.len() / r.ch) as i64;
    // Gap of more than ~20 ms since the last data: fill with silence.
    if chunk_start - have > (r.rate / 50) as i64 {
        let pad = (chunk_start - have) as usize * r.ch;
        let n = r.data.len() + pad;
        r.data.resize(n, 0.0);
    }
    r.data.extend_from_slice(chunk);
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 3 {
        eprintln!("usage: loopback OUT.wav SECONDS");
        std::process::exit(2);
    }
    let out = &args[1];
    let secs: f64 = args[2].parse().expect("SECONDS must be a number");
    let host = cpal::default_host();
    let dev = host.default_output_device().expect("no default output device");
    let cfg = dev.default_output_config().expect("no output config");
    let name = dev.description().map(|d| d.name().to_string()).unwrap_or_else(|_| "?".into());
    let rate = cfg.sample_rate();
    let ch = cfg.channels() as usize;
    let fmt = cfg.sample_format();
    let sc: cpal::StreamConfig = cfg.clone().into();
    let rec = Arc::new(Mutex::new(Rec { data: Vec::new(), start: Instant::now(), rate, ch }));
    let err = |e| eprintln!("stream error: {e}");
    let r2 = rec.clone();
    let stream = match fmt {
        cpal::SampleFormat::F32 => dev.build_input_stream(&sc, move |d: &[f32], _| push(&r2, d), err, None),
        cpal::SampleFormat::I16 => dev.build_input_stream(&sc, move |d: &[i16], _| {
            let v: Vec<f32> = d.iter().map(|&s| s as f32 / 32768.0).collect();
            push(&r2, &v)
        }, err, None),
        other => panic!("unsupported sample format {other:?}"),
    }
    .expect("cannot open loopback stream");
    let wall = local_time();
    rec.lock().unwrap().start = Instant::now();
    stream.play().expect("play");
    println!("started {wall}  device \"{name}\"  {rate} Hz  {ch} ch  {fmt:?}");
    std::thread::sleep(Duration::from_secs_f64(secs));
    drop(stream);
    let mut r = rec.lock().unwrap();
    let total = (secs * rate as f64) as usize * ch;
    r.data.resize(total, 0.0);
    write_wav(out, &r.data, rate, ch as u16).expect("write wav");
    println!("done: {out} ({secs} s)");
}

fn write_wav(path: &str, d: &[f32], rate: u32, ch: u16) -> std::io::Result<()> {
    use std::io::Write;
    let bytes = (d.len() * 4) as u32;
    let mut f = std::io::BufWriter::new(std::fs::File::create(path)?);
    f.write_all(b"RIFF")?;
    f.write_all(&(36 + bytes).to_le_bytes())?;
    f.write_all(b"WAVEfmt ")?;
    f.write_all(&16u32.to_le_bytes())?;
    f.write_all(&3u16.to_le_bytes())?;
    f.write_all(&ch.to_le_bytes())?;
    f.write_all(&rate.to_le_bytes())?;
    f.write_all(&(rate * ch as u32 * 4).to_le_bytes())?;
    f.write_all(&(ch * 4).to_le_bytes())?;
    f.write_all(&32u16.to_le_bytes())?;
    f.write_all(b"data")?;
    f.write_all(&bytes.to_le_bytes())?;
    for s in d {
        f.write_all(&s.to_le_bytes())?;
    }
    f.flush()
}

#[repr(C)]
struct SystemTime {
    y: u16, mo: u16, dow: u16, d: u16, h: u16, mi: u16, s: u16, ms: u16,
}
extern "system" {
    fn GetLocalTime(t: *mut SystemTime);
}
fn local_time() -> String {
    let mut t = SystemTime { y: 0, mo: 0, dow: 0, d: 0, h: 0, mi: 0, s: 0, ms: 0 };
    unsafe { GetLocalTime(&mut t) };
    format!("{:02}:{:02}:{:02}.{:03}", t.h, t.mi, t.s, t.ms)
}
