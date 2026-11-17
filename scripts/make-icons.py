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
    w = max(2, int(84 * k))
    # 双向箭头 = 两端互通
    def p(x, y):
        return (x * k, y * k)
    d.line([p(250, 390), p(774, 390)], fill=fg, width=w)
    d.line([p(250, 634), p(774, 634)], fill=fg, width=w)
    d.line([p(400, 240), p(250, 390), p(400, 540)], fill=fg, width=w, joint='curve')
    d.line([p(624, 484), p(774, 634), p(624, 784)], fill=fg, width=w, joint='curve')
    for x, y in ((250, 390), (774, 634), (400, 240), (400, 540), (624, 484), (624, 784), (774, 390), (250, 634)):
        r = w / 2
        d.ellipse([x * k - r, y * k - r, x * k + r, y * k + r], fill=fg)
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
