"""生成托盘三态图标与应用图标源图（1024px），应用图标再用 `npx tauri icon` 生成全套。"""
import sys
from pathlib import Path
from PIL import Image, ImageDraw

out = Path(sys.argv[1])
S = 1024


def glyph(size, bg, fg, dot=None):
    im = Image.new('RGBA', (size, size), (0, 0, 0, 0))
    d = ImageDraw.Draw(im)
    k = size / 1024
    d.rounded_rectangle([40 * k, 40 * k, 984 * k, 984 * k], radius=230 * k, fill=bg)
    w = max(2, int(78 * k))
    # 两个相向的半弧 = 桥
    d.arc([200 * k, 300 * k, 620 * k, 720 * k], 110, 250, fill=fg, width=w)
    d.arc([404 * k, 300 * k, 824 * k, 720 * k], -70, 70, fill=fg, width=w)
    d.line([330 * k, 512 * k, 694 * k, 512 * k], fill=fg, width=w)
    for cx in (330, 694):
        r = 64 * k
        d.ellipse([cx * k - r, 512 * k - r, cx * k + r, 512 * k + r], fill=fg)
    if dot:
        r = 190 * k
        cx, cy = 800 * k, 800 * k
        d.ellipse([cx - r - 40 * k, cy - r - 40 * k, cx + r + 40 * k, cy + r + 40 * k], fill=(0, 0, 0, 0))
        d.ellipse([cx - r, cy - r, cx + r, cy + r], fill=dot)
    return im


BG = (24, 24, 32, 255)
FG = (236, 236, 244, 255)
glyph(S, BG, FG).save(out / 'app-icon.png')
for name, color in (('ok', (34, 197, 94, 255)), ('warn', (245, 158, 11, 255)), ('err', (239, 68, 68, 255))):
    big = glyph(S, BG, FG, color)
    big.resize((64, 64), Image.LANCZOS).save(out / f'tray-{name}.png')
print('ok')
