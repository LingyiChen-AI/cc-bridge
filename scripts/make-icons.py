"""生成应用图标源图（1024px）与托盘三态图标。应用图标再用 `npx tauri icon` 生成全套。
设计（③ 双箭头转发）：黑色圆角方块，白色 >（Codex）接着陶土橙 >（Claude Code）和一个橙色光标块，
表示请求从 Codex 一侧转到 Claude Code 一侧。只用颜色和通用的终端符号，不使用任何官方标志。"""
import sys
from pathlib import Path
from PIL import Image, ImageDraw

out = Path(sys.argv[1])
BG = (20, 20, 20, 255)
WHITE = (255, 255, 255, 255)
ORANGE = (217, 119, 87, 255)


def chevron(d, pts, width, color):
    d.line(pts, fill=color, width=width, joint='curve')
    r = width / 2
    for x, y in (pts[0], pts[2]):
        d.ellipse([x - r, y - r, x + r, y + r], fill=color)


def logo(size, dot=None, bold=1.0):
    big = 4
    W = size * big
    k = W / 100  # 按 100 单位网格绘制，与界面里的 SVG 一致
    im = Image.new('RGBA', (W, W), (0, 0, 0, 0))
    d = ImageDraw.Draw(im)
    d.rounded_rectangle([4 * k, 4 * k, 96 * k, 96 * k], radius=22 * k, fill=BG)
    w = int(10 * bold * k)
    chevron(d, [(22 * k, 30 * k), (42 * k, 50 * k), (22 * k, 70 * k)], w, WHITE)
    chevron(d, [(48 * k, 30 * k), (68 * k, 50 * k), (48 * k, 70 * k)], w, ORANGE)
    if bold <= 1.0:
        d.rounded_rectangle([72 * k, 62 * k, 82 * k, 70 * k], radius=3 * k, fill=ORANGE)
    if dot:
        r = 15 * k
        cx = cy = 82 * k
        d.ellipse([cx - r - 4 * k, cy - r - 4 * k, cx + r + 4 * k, cy + r + 4 * k], fill=(0, 0, 0, 0))
        d.ellipse([cx - r, cy - r, cx + r, cy + r], fill=dot)
    return im.resize((size, size), Image.LANCZOS)


logo(1024).save(out / 'app-icon.png')
for name, color in (('ok', (34, 197, 94, 255)), ('warn', (245, 158, 11, 255)), ('err', (239, 68, 68, 255))):
    logo(256, color, bold=1.2).resize((64, 64), Image.LANCZOS).save(out / f'tray-{name}.png')
print('ok')
