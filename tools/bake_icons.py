import subprocess, os, math
from PIL import Image

# Fixed icon order — this IS the IconId index mapping. Keep in sync with font.rs.
ORDER = [
    "box","move","rotate-3d","scale-3d","mouse-pointer-2","grid-3x3","eye","eye-off",
    "trash-2","copy","undo-2","redo-2","plus","minus","layers","settings",
    "ruler","axis-3d","grip","pen-tool",
]
ICON = 16
COLS = 8
rows = math.ceil(len(ORDER)/COLS)
W, H = COLS*ICON, rows*ICON
sheet = Image.new("L", (W, H), 0)
src = "assets/icons/lucide"
tmp = "/tmp/_icontmp"; os.makedirs(tmp, exist_ok=True)
missing = []
for i, name in enumerate(ORDER):
    svg = os.path.join(src, name+".svg")
    if not os.path.isfile(svg):
        missing.append(name); continue
    png = os.path.join(tmp, name+".png")
    subprocess.run(["rsvg-convert","-w",str(ICON),"-h",str(ICON),svg,"-o",png], check=True)
    im = Image.open(png).convert("RGBA")
    alpha = im.split()[3]                      # alpha = ink coverage (mask)
    col, row = i % COLS, i // COLS
    sheet.paste(alpha, (col*ICON, row*ICON))
if missing:
    print("MISSING:", missing); raise SystemExit(1)
# Raw R8 (row-major), plus a PNG for inspection.
with open("assets/icons/ui_icons.r8","wb") as f:
    f.write(sheet.tobytes())
sheet.save("assets/icons/ui_icons.png")
print(f"baked {len(ORDER)} icons -> {W}x{H} R8 ({W*H} bytes), {sum(1 for p in sheet.getdata() if p>0)} ink px")
