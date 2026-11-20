"""生成应用图标源图（1024px）与托盘三态图标。应用图标再用 `npx tauri icon` 生成全套。
设计（③ 双箭头转发）：黑色圆角方块，白色 >（Codex）接着陶土橙 >（Claude Code）和一个橙色光标块，
表示请求从 Codex 一侧转到 Claude Code 一侧。只用颜色和通用的终端符号，不使用任何官方标志。

两个源图，因为两边的图标网格约定相反：
- app-icon.png        满版（图形占画布 92%），给 Windows / Linux 用，那边的图标基本铺满画布
- app-icon-macos.png  Apple 图标网格（1024 画布里 824 的圆角方块、圆角半径 185.4、四边各留 100），
                      给 icon.icns 用。满版图形会让 Dock 和 Cmd+Tab 里比邻居大一圈——
                      量过：满版 0.926，Apple 规范 0.805，ChatGPT 0.861，QQ 0.811。

重新生成全套（icon.icns 只能来自 macOS 那张，别混）：

    python3 scripts/make-icons.py src-tauri/icons
    npx tauri icon src-tauri/icons/app-icon.png -o src-tauri/icons        # Windows / Linux 全套
    npx tauri icon src-tauri/icons/app-icon-macos.png -o /tmp/icons-mac   # 只取 icon.icns
    cp /tmp/icons-mac/icon.icns src-tauri/icons/icon.icns
"""
import sys
from pathlib import Path
from PIL import Image, ImageDraw

out = Path(sys.argv[1])
BG = (20, 20, 20, 255)
WHITE = (255, 255, 255, 255)
ORANGE = (217, 119, 87, 255)

# 以 100 单位网格表示的「图形方块」：边距与圆角半径
FULL = (4.0, 22.0)
APPLE = (100 / 1024 * 100, 185.4 / 1024 * 100)  # ≈ (9.766, 18.105)


def chevron(d, pts, width, color):
    d.line(pts, fill=color, width=width, joint='curve')
    r = width / 2
    for x, y in (pts[0], pts[2]):
        d.ellipse([x - r, y - r, x + r, y + r], fill=color)


def logo(size, dot=None, bold=1.0, box=FULL):
    big = 4
    W = size * big
    k = W / 100  # 按 100 单位网格绘制，与界面里的 SVG 一致
    margin, radius = box
    # 图案原本是照着 4..96 这个方块画的；换边距时整体等比缩放，画面内部比例不变
    scale = (100 - 2 * margin) / 92
    p = lambda g: (margin + (g - 4) * scale) * k
    im = Image.new('RGBA', (W, W), (0, 0, 0, 0))
    d = ImageDraw.Draw(im)
    d.rounded_rectangle([p(4), p(4), p(96), p(96)], radius=radius * k, fill=BG)
    w = int(10 * bold * scale * k)
    chevron(d, [(p(22), p(30)), (p(42), p(50)), (p(22), p(70))], w, WHITE)
    chevron(d, [(p(48), p(30)), (p(68), p(50)), (p(48), p(70))], w, ORANGE)
    if bold <= 1.0:
        d.rounded_rectangle([p(72), p(62), p(82), p(70)], radius=3 * scale * k, fill=ORANGE)
    if dot:
        r = 15 * scale * k
        pad = 4 * scale * k
        cx = cy = p(82)
        d.ellipse([cx - r - pad, cy - r - pad, cx + r + pad, cy + r + pad], fill=(0, 0, 0, 0))
        d.ellipse([cx - r, cy - r, cx + r, cy + r], fill=dot)
    return im.resize((size, size), Image.LANCZOS)


logo(1024).save(out / 'app-icon.png')
logo(1024, box=APPLE).save(out / 'app-icon-macos.png')
for name, color in (('ok', (34, 197, 94, 255)), ('warn', (245, 158, 11, 255)), ('err', (239, 68, 68, 255))):
    logo(256, color, bold=1.2).resize((64, 64), Image.LANCZOS).save(out / f'tray-{name}.png')
print('ok')
