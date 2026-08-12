"""Volumetric exactly-looping cumulus deck.

Dimension budget problem: Blender's noise is at most 4D, but a volume needs 3
spatial dims, leaving only W for time -- and a 1D time path cannot be a closed
circle (w = r*sin(2*pi*t) closes, but ping-pongs).

Solution used here: the *coverage* field is 2D-in-space (x, y), so dims 3 and 4
are free for a genuine circle -> Vector = (x, y, r*cos(2*pi*t)), W = r*sin(2*pi*t).
Vertical structure comes from an analytic height profile (and a static 3D
erosion noise), not from the animated noise. Loop is therefore exact AND the
time path has constant speed (no ping-pong).
"""
import bpy, sys, math, os, time

argv = sys.argv[sys.argv.index("--") + 1:] if "--" in sys.argv else []
def arg(n, d): return argv[argv.index(n) + 1] if n in argv else d

ENGINE  = arg("--engine", "CYCLES")
DEVICE  = arg("--device", "CPU")
OUT     = arg("--out", "./vol/")
RX, RY  = int(arg("--rx", "960")), int(arg("--ry", "540"))
SAMPLES = int(arg("--samples", "64"))
LOOP    = int(arg("--loop", "180"))
FRAMES  = [int(f) for f in arg("--frames", "0").split(",")]

bpy.ops.wm.read_factory_settings(use_empty=True)
sc = bpy.context.scene
sc.render.engine = ENGINE
sc.render.resolution_x, sc.render.resolution_y = RX, RY
sc.render.image_settings.file_format = "PNG"
sc.render.image_settings.color_mode = "BW"
sc.view_settings.view_transform = "Standard"
sc.frame_start, sc.frame_end = 0, LOOP - 1

if ENGINE == "CYCLES":
    if DEVICE == "GPU":
        prefs = bpy.context.preferences.addons["cycles"].preferences
        prefs.compute_device_type = "METAL"   # CUDA/OPTIX/HIP/ONEAPI elsewhere
        prefs.get_devices()
        for d in prefs.devices:
            d.use = (d.type != "CPU")
        sc.cycles.device = "GPU"
    sc.cycles.samples = SAMPLES
    sc.cycles.use_adaptive_sampling = False
    sc.cycles.use_denoising = True
    sc.cycles.seed = 0
    sc.cycles.use_animated_seed = False       # CRITICAL for an exact loop
    sc.cycles.volume_step_rate = float(arg("--steprate","4.0"))
    sc.cycles.volume_max_steps = 256
    sc.cycles.max_bounces = 4
    sc.cycles.volume_bounces = 4
else:
    sc.eevee.taa_render_samples = int(arg("--samples","64"))
    sc.eevee.volumetric_samples = 128
    sc.eevee.volumetric_start = 1.0
    sc.eevee.volumetric_end = 200.0
    sc.eevee.volumetric_tile_size = "2"
    sc.eevee.use_volumetric_shadows = True

cd = bpy.data.cameras.new("C"); cd.type = "ORTHO"; cd.ortho_scale = 34.0
cam = bpy.data.objects.new("C", cd)
cam.location = (0, -60, 3); cam.rotation_euler = (math.radians(88), 0, 0)
sc.collection.objects.link(cam); sc.camera = cam

sun_d = bpy.data.lights.new("S", "SUN"); sun_d.energy = 8.0; sun_d.angle = 0.02
sun = bpy.data.objects.new("S", sun_d)
sun.rotation_euler = (math.radians(45), 0, math.radians(150))
sc.collection.objects.link(sun)

world = bpy.data.worlds.new("W"); sc.world = world; world.use_nodes = True
world.node_tree.nodes["Background"].inputs[0].default_value = (0.45, 0.45, 0.45, 1)

bpy.ops.mesh.primitive_cube_add(size=2)
dom = bpy.context.object
dom.scale = (24, 24, 4)

mat = bpy.data.materials.new("Vol"); mat.use_nodes = True
nt = mat.node_tree; nt.nodes.clear()
N, L = nt.nodes.new, nt.links.new

def m(op, a=None, b=None, va=0.0, vb=0.0, clamp=False):
    n = N("ShaderNodeMath"); n.operation = op; n.use_clamp = clamp
    if a is not None: L(a, n.inputs[0])
    else: n.inputs[0].default_value = va
    if b is not None: L(b, n.inputs[1])
    else: n.inputs[1].default_value = vb
    return n.outputs[0]

tv = N("ShaderNodeValue")
tv.outputs[0].default_value = 0.0; tv.outputs[0].keyframe_insert("default_value", frame=0)
tv.outputs[0].default_value = 1.0; tv.outputs[0].keyframe_insert("default_value", frame=LOOP)
for fc in mat.node_tree.animation_data.action.fcurves:
    for k in fc.keyframe_points: k.interpolation = "LINEAR"

ang  = m("MULTIPLY", a=tv.outputs[0], vb=2 * math.pi)
cosA = m("COSINE", a=ang)
sinA = m("SINE", a=ang)

tc = N("ShaderNodeTexCoord")
sep = N("ShaderNodeSeparateXYZ"); L(tc.outputs["Object"], sep.inputs["Vector"])
DR, EV = 0.35, 0.18

# --- coverage coordinate: (x, y, r*cos) with W = r*sin  -> closed circle in time
cov = N("ShaderNodeCombineXYZ")
L(m("ADD", a=sep.outputs["X"], b=m("MULTIPLY", a=cosA, vb=DR)), cov.inputs["X"])
L(m("ADD", a=sep.outputs["Y"], b=m("MULTIPLY", a=sinA, vb=DR)), cov.inputs["Y"])
L(m("MULTIPLY", a=cosA, vb=EV), cov.inputs["Z"])
COVW = m("MULTIPLY", a=sinA, vb=EV)

def noise(vec, wsock, dims, scale, det, rough, dist=0.0, lac=2.0):
    n = N("ShaderNodeTexNoise"); n.noise_dimensions = dims; n.normalize = True
    L(vec, n.inputs["Vector"])
    if wsock is not None: L(wsock, n.inputs["W"])
    n.inputs["Scale"].default_value = scale
    n.inputs["Detail"].default_value = det
    n.inputs["Roughness"].default_value = rough
    n.inputs["Lacunarity"].default_value = lac
    n.inputs["Distortion"].default_value = dist
    return n.outputs["Fac"]

coverage = noise(cov.outputs["Vector"], COVW, "4D", 0.10, 7.0, 0.55, dist=0.3)

vor = N("ShaderNodeTexVoronoi")
vor.voronoi_dimensions = "4D"; vor.feature = "SMOOTH_F1"
L(cov.outputs["Vector"], vor.inputs["Vector"]); L(COVW, vor.inputs["W"])
vor.inputs["Scale"].default_value = 0.16
vor.inputs["Smoothness"].default_value = 0.4
billow = m("SUBTRACT", va=1.0, b=m("MULTIPLY", a=vor.outputs["Distance"], vb=1.4), clamp=True)

cov2 = m("ADD", a=m("MULTIPLY", a=coverage, vb=0.55), b=m("MULTIPLY", a=billow, vb=0.45))

# --- vertical cumulus profile: flat base, domed top, height scales with coverage
zn = m("DIVIDE", a=m("ADD", a=sep.outputs["Z"], vb=1.0), vb=2.0)          # 0..1
top = m("MULTIPLY", a=cov2, vb=1.3)
prof = m("SUBTRACT", va=1.0, b=m("DIVIDE", a=zn, b=m("MAXIMUM", a=top, vb=0.001)), clamp=True)
prof = m("POWER", a=prof, vb=0.55)
base_cut = m("SMOOTH_MIN", a=zn, vb=1.0) if False else zn                 # flat base at z=0

# --- static 3D erosion detail (world-locked; does not need to loop)
erode = noise(tc.outputs["Object"], None, "3D", 0.55, 6.0, 0.65)

density_raw = m("SUBTRACT",
                a=m("MULTIPLY", a=cov2, b=prof),
                b=m("MULTIPLY", a=m("SUBTRACT", a=erode, vb=0.5), vb=0.16))

mr = N("ShaderNodeMapRange"); mr.interpolation_type = "SMOOTHSTEP"; mr.clamp = True
L(density_raw, mr.inputs["Value"])
mr.inputs["From Min"].default_value = 0.30
mr.inputs["From Max"].default_value = 0.44
dens = m("MULTIPLY", a=mr.outputs["Result"], vb=4.0)

pv = N("ShaderNodeVolumePrincipled")
L(dens, pv.inputs["Density"])
pv.inputs["Anisotropy"].default_value = 0.4
out = N("ShaderNodeOutputMaterial")
L(pv.outputs["Volume"], out.inputs["Volume"])
dom.data.materials.append(mat)

os.makedirs(OUT, exist_ok=True)
for f in FRAMES:
    sc.frame_set(f)
    sc.render.filepath = os.path.join(OUT, f"f{f:04d}")
    t0 = time.time()
    bpy.ops.render.render(write_still=True)
    print(f"RENDERTIME frame {f}: {time.time()-t0:.2f}s")
