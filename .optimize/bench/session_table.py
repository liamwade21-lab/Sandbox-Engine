"""python -I .optimize/bench/session_table.py FILE.perf [all]: one short line per segment of a session test.
Frame median, the CPU time of the main schedules, the render thread's preparation and its render graph (command buffers finished and submitted), the GPU's own time (all top-level passes), the opaque pass, then the counts that must stay flat.
Only the measuring segments (names not starting with 'trip') unless 'all' is given."""
import re, sys

for l in open(sys.argv[1]):
    f = l.rstrip().split('\t')
    if f[0].startswith(('trip', 'v_')) and 'all' not in sys.argv[2:]:
        continue
    kv = {x.split(' ')[0]: x for x in f[1:]}
    med = f[3].split(' ')[1]
    gpu = {x.rpartition(' ')[0]: float(x.rpartition(' ')[2]) for x in f if x.count('/') == 1 and 'elapsed_gpu' in x}
    tot = sum(gpu.values())
    nums = lambda k: '/'.join(re.findall(r'\d+', kv.get(k, '')))
    cpu = lambda k: kv.get(k, ' ?').split(' ')[1]
    print(f"{f[0]:8} med {med:>6} cpu {cpu('cpu_main'):>5}/{cpu('cpu_render'):>5}/{cpu('cpu_graph'):>5} gpu {tot:5.2f} opq {gpu.get('main_opaque_pass_3d/elapsed_gpu', 0):5.2f} | "
          f"ent {nums('all_entities')} | wgpu {nums('wgpu')} | rend {nums('render')} | img {nums('images')} "
          f"| mesh {nums('meshes')} | gpuimg {nums('gpu_images')} | ram {nums('ram')}")
