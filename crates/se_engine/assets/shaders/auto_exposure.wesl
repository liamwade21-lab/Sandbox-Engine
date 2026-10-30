// SE's auto exposure (`exposure.rs`): hdr_histogram.glsl's centre-masked log2 luminance histogram and
// FUN_1404c09a0's percentile median and adaptation. Runs on the HDR frame before tone mapping.

struct Params {
    // x, y: log2 luminance of bins 0 and 256 (MinExposureComp, MaxExposureComp); z, w: low and high fractions
    range: vec4<f32>,
    // x: log2 AutoExpTargetBright, y, z: AutoExpMin/MaxExpComp, w: frame time (s)
    aim: vec4<f32>,
    // x: AutoExpSpeedUp, y: AutoExpSpeedDown, z: 1 / AutoExpMaskSize^2, w: AutoExpMaskValue
    speed: vec4<f32>,
};

@group(0) @binding(0) var frame: texture_2d<f32>;
@group(0) @binding(1) var<uniform> p: Params;
@group(0) @binding(2) var<storage, read_write> bins: array<atomic<u32>, 256>;
// x: the compensation (stops, positive darker), y: the metered median log2 luminance
@group(0) @binding(3) var<storage, read_write> state: vec4<f32>;

const BINS: u32 = 256u;
// SE meters a mip level of the frame; the port averages 16 x 16 pixel blocks.
const BLOCK: u32 = 16u;

var<workgroup> local_bins: array<atomic<u32>, 256>;

@compute @workgroup_size(16, 16, 1)
fn histogram(@builtin(global_invocation_id) gid: vec3<u32>, @builtin(local_invocation_index) li: u32) {
    atomicStore(&local_bins[li], 0u);
    workgroupBarrier();
    let dim = textureDimensions(frame);
    let cells = (dim + vec2<u32>(BLOCK - 1u)) / BLOCK;
    if gid.x < cells.x && gid.y < cells.y {
        var sum = vec3<f32>(0.0);
        var n = 0.0;
        for (var y = 0u; y < BLOCK; y++) {
            for (var x = 0u; x < BLOCK; x++) {
                let px = gid.xy * BLOCK + vec2<u32>(x, y);
                if px.x < dim.x && px.y < dim.y {
                    sum += textureLoad(frame, px, 0).rgb;
                    n += 1.0;
                }
            }
        }
        let lum = clamp(dot(sum / max(n, 1.0), vec3<f32>(0.2126, 0.7152, 0.0722)), 0.0, 65000.0);
        // hdr_histogram.glsl's metering zone: a quadric falloff from the centre.
        let m = (vec2<f32>(gid.xy) + 0.5) / vec2<f32>(cells) * 2.0 - 1.0;
        let mask = p.speed.w * sqrt(clamp(1.0 - p.speed.z * dot(m, m), 0.0, 1.0));
        let l = lum * mask;
        var bin = 0u;
        if l > 0.0 {
            let t = clamp((log2(l) - p.range.x) / (p.range.y - p.range.x), 0.0, 1.0);
            bin = min(u32(t * f32(BINS)), BINS - 1u);
        }
        atomicAdd(&local_bins[bin], 1u);
    }
    workgroupBarrier();
    let v = atomicLoad(&local_bins[li]);
    if v > 0u {
        atomicAdd(&bins[li], v);
    }
}

// FUN_1404c09a0: bin 0 is left out; the low and high percentiles bound the bins whose median is metered.
@compute @workgroup_size(1, 1, 1)
fn adapt() {
    var total = 0u;
    for (var i = 1u; i < BINS; i++) {
        total += atomicLoad(&bins[i]);
    }
    let low = f32(total) * p.range.z;
    let high = f32(total) * p.range.w;
    var cum = 0u;
    var i_low = -1;
    var i_high = -1;
    for (var i = 1u; i < BINS; i++) {
        if i_low >= 0 && i_high >= 0 {
            break;
        }
        cum += atomicLoad(&bins[i]);
        if f32(cum) >= low && i_low < 0 {
            i_low = i32(i);
        }
        if f32(cum) >= high && i_high < 0 {
            i_high = i32(i);
        }
    }
    var i_med = i_low;
    if i_low >= 0 && i_high >= i_low {
        var part = 0u;
        for (var i = i_low; i <= i_high; i++) {
            part += atomicLoad(&bins[u32(i)]);
        }
        let half = f32(part) * 0.5;
        var c = 0u;
        for (var i = i_low; i <= i_high; i++) {
            c += atomicLoad(&bins[u32(i)]);
            if f32(c) >= half {
                i_med = i;
                break;
            }
        }
    }
    for (var i = 0u; i < BINS; i++) {
        atomicStore(&bins[i], 0u);
    }
    let med = p.range.x + (p.range.y - p.range.x) * f32(max(i_med, 0)) / f32(BINS);
    let goal = clamp(med - p.aim.x, p.aim.y, p.aim.z);
    let comp = state.x;
    let speed = select(p.speed.y, p.speed.x, goal > comp);
    let f = clamp(1.0 - exp(-p.aim.w * speed), 0.0, 1.0);
    state = vec4<f32>(comp + (goal - comp) * f, med, 0.0, 0.0);
}
