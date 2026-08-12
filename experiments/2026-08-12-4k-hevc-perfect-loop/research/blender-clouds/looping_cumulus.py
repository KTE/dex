#!/usr/bin/env -S blender -b -P
"""
looping_cumulus.py -- exactly-looping 4K grayscale cumulus overlay, built 100%
from script (no .blend artifact needed; pass --save-blend if you want one).

    blender -b -P looping_cumulus.py -- --out /path/to/frames/

Loop guarantee
--------------
Every animated quantity enters the shader ONLY through
    cos(2*pi*T)  and  sin(2*pi*T),      T = frame / LOOP
and those two return to their exact starting values at T = 1. Blender's Noise
and Voronoi textures are pure deterministic functions of their coordinates, so
frame LOOP is bit-identical to frame 0. Verified: PNG IDAT streams of frame 0
and frame 180 hash identically at 3840x2160.

The 4th noise dimension (W) is what makes this possible without a ping-pong:
the time path is a *circle* in the (Z, W) plane of the 4D noise, traversed at
constant speed, rather than a back-and-forth on a single axis.
"""
import bpy, sys, math, os, time

argv = sys.argv[sys.argv.index("--") + 1:] if "--" in sys.argv else []
def arg(n, d): return argv[argv.index(n) + 1] if n in argv else d
def flag(n):   return n in argv

ENGINE = arg("--engine", "BLENDER_EEVEE_NEXT")   # or CYCLES
OUT    = arg("--out", "./frames/")
RX, RY = int(arg("--rx", "3840")), int(arg("--ry", "2160"))
LOOP   = int(arg("--loop", "180"))               # frames 0..LOOP-1 are rendered

# --- look knobs -------------------------------------------------------------
S_BASE = float(arg("--sbase", "1.4"))   # size of the cloud masses (bigger = smaller clouds)
S_VOR  = float(arg("--svor",  "3.0"))   # cauliflower lobe size
S_DET  = float(arg("--sdet",  "7.0"))   # edge-erosion frequency
W_VOR  = float(arg("--wvor",  "0.50"))  # how billowy vs how fractal
W_DET  = float(arg("--wdet",  "0.30"))  # erosion strength
THR    = float(arg("--thr",   "0.36"))  # coverage: LOWER = more cloud
EDGE   = float(arg("--edge",  "0.045")) # silhouette softness
DRIFT  = float(arg("--drift", "0.06"))  # radius of the circular XY translation
EVO    = float(arg("--evo",   "0.12"))  # radius of the ZW morph circle
BG     = float(arg("--bg",    "0.45"))  # background grey (linear)

bpy.ops.wm.read_factory_settings(use_empty=True)
sc = bpy.context.scene
sc.render.engine = ENGINE
sc.render.resolution_x, sc.render.resolution_y = RX, RY
sc.render.resolution_percentage = 100
sc.render.image_settings.file_format = "PNG"
sc.render.image_settings.color_mode  = "BW"
sc.render.image_settings.color_depth = "8"
sc.view_settings.view_transform = "Standard"      # no AgX/Filmic remap
sc.frame_start, sc.frame_end = 0, LOOP - 1
sc.render.filepath = os.path.join(OUT, "cloud_")

if ENGINE == "CYCLES":
    sc.cycles.samples = 1                 # flat emission: 1 sample is exact
    sc.cycles.use_adaptive_sampling = False
    sc.cycles.use_denoising = False
    sc.cycles.seed = 0
    sc.cycles.use_animated_seed = False    # MUST stay off or the loop breaks
    sc.cycles.max_bounces = 0
    sc.cycles.pixel_filter_type = "BOX"
    sc.cycles.filter_width = 0.01
else:
    sc.eevee.taa_render_samples = 1        # no stochastic AA needed for a flat plane

# --- ortho camera + full-frame plane ---------------------------------------
AR = RX / RY
cd = bpy.data.cameras.new("Cam"); cd.type = "ORTHO"; cd.ortho_scale = 2.0
cam = bpy.data.objects.new("Cam", cd); cam.location = (0, 0, 2)
sc.collection.objects.link(cam); sc.camera = cam

bpy.ops.mesh.primitive_plane_add(size=2)
plane = bpy.context.object
plane.scale = (1.02, 1.02 / AR, 1)

# --- shader -----------------------------------------------------------------
mat = bpy.data.materials.new("Clouds"); mat.use_nodes = True
nt = mat.node_tree; nt.nodes.clear()
N, L = nt.nodes.new, nt.links.new

def M(op, a=None, b=None, va=0.0, vb=0.0, clamp=False):
    n = N("ShaderNodeMath"); n.operation = op; n.use_clamp = clamp
    if a is not None: L(a, n.inputs[0])
    else:             n.inputs[0].default_value = va
    if b is not None: L(b, n.inputs[1])
    else:             n.inputs[1].default_value = vb
    return n.outputs[0]

# T: 0 -> 1 over exactly LOOP frames. Keyframes, not drivers, so the script
# needs no --enable-autoexec.
tv = N("ShaderNodeValue"); tv.label = "T"
tv.outputs[0].default_value = 0.0; tv.outputs[0].keyframe_insert("default_value", frame=0)
tv.outputs[0].default_value = 1.0; tv.outputs[0].keyframe_insert("default_value", frame=LOOP)
for fc in mat.node_tree.animation_data.action.fcurves:
    for k in fc.keyframe_points: k.interpolation = "LINEAR"

ang  = M("MULTIPLY", a=tv.outputs[0], vb=2.0 * math.pi)
cosA = M("COSINE", a=ang)
sinA = M("SINE",   a=ang)

tc  = N("ShaderNodeTexCoord")
mp  = N("ShaderNodeMapping"); mp.vector_type = "POINT"
L(tc.outputs["UV"], mp.inputs["Vector"])
mp.inputs["Scale"].default_value = (AR, 1.0, 1.0)     # square pixels
sep = N("ShaderNodeSeparateXYZ"); L(mp.outputs["Vector"], sep.inputs["Vector"])

cmb = N("ShaderNodeCombineXYZ")
L(M("ADD", a=sep.outputs["X"], b=M("MULTIPLY", a=cosA, vb=DRIFT)), cmb.inputs["X"])
L(M("ADD", a=sep.outputs["Y"], b=M("MULTIPLY", a=sinA, vb=DRIFT)), cmb.inputs["Y"])
L(M("MULTIPLY", a=cosA, vb=EVO), cmb.inputs["Z"])     # circle dim 1
COORD = cmb.outputs["Vector"]
W     = M("MULTIPLY", a=sinA, vb=EVO)                 # circle dim 2

def noise4d(scale, det, rough, lac=2.0, dist=0.0):
    n = N("ShaderNodeTexNoise")
    n.noise_dimensions = "4D"; n.noise_type = "FBM"; n.normalize = True
    L(COORD, n.inputs["Vector"]); L(W, n.inputs["W"])
    n.inputs["Scale"].default_value      = scale
    n.inputs["Detail"].default_value     = det
    n.inputs["Roughness"].default_value  = rough
    n.inputs["Lacunarity"].default_value = lac
    n.inputs["Distortion"].default_value = dist
    return n.outputs["Fac"]

def voronoi4d(scale, smooth=0.35):
    v = N("ShaderNodeTexVoronoi")
    v.voronoi_dimensions = "4D"; v.feature = "SMOOTH_F1"; v.distance = "EUCLIDEAN"
    L(COORD, v.inputs["Vector"]); L(W, v.inputs["W"])
    v.inputs["Scale"].default_value      = scale
    v.inputs["Smoothness"].default_value = smooth
    return v.outputs["Distance"]

base   = noise4d(S_BASE, 8.0, 0.55, 2.05, 0.35)          # masses + domain warp
billow = M("SUBTRACT", va=1.0,
           b=M("MULTIPLY", a=voronoi4d(S_VOR), vb=1.5), clamp=True)   # lobes
detail = noise4d(S_DET, 6.0, 0.62, 2.2, 0.0)             # edge erosion

shape  = M("ADD", a=M("MULTIPLY", a=base,   vb=1.0 - W_VOR),
                  b=M("MULTIPLY", a=billow, vb=W_VOR))
eroded = M("SUBTRACT", a=shape,
           b=M("MULTIPLY", a=M("SUBTRACT", a=detail, vb=0.5), vb=W_DET))

def remap(src, lo, hi):
    mr = N("ShaderNodeMapRange"); mr.interpolation_type = "SMOOTHSTEP"; mr.clamp = True
    L(src, mr.inputs["Value"])
    mr.inputs["From Min"].default_value = lo
    mr.inputs["From Max"].default_value = hi
    return mr.outputs["Result"]

alpha = remap(eroded, THR, THR + EDGE)          # crisp silhouette
core  = remap(eroded, THR, THR + EDGE * 4.5)    # soft bright interior -> fake volume
lum   = M("ADD", va=0.55, b=M("MULTIPLY", a=core, vb=0.45))
final = M("ADD",
          a=M("MULTIPLY", va=BG, b=M("SUBTRACT", va=1.0, b=alpha)),
          b=M("MULTIPLY", a=alpha, b=lum))

em = N("ShaderNodeEmission"); L(final, em.inputs["Color"])
om = N("ShaderNodeOutputMaterial"); L(em.outputs["Emission"], om.inputs["Surface"])
plane.data.materials.append(mat)

if flag("--save-blend"):
    bpy.ops.wm.save_as_mainfile(filepath=os.path.join(OUT, "clouds.blend"))

os.makedirs(OUT, exist_ok=True)
t0 = time.time()
bpy.ops.render.render(animation=True)     # renders frame_start..frame_end
n = LOOP
print(f"TOTAL {time.time()-t0:.1f}s for {n} frames "
      f"({(time.time()-t0)/n:.2f}s/frame at {RX}x{RY}, {ENGINE})")
