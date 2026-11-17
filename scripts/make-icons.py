"""生成应用图标源图（1024px）与托盘三态图标。应用图标再用 `npx tauri icon` 生成全套。
设计：靛蓝→青渐变圆角方块，白色 S 形路由线从空心起点连到实心终点，表示把请求从一端转到另一端。"""
import sys
from pathlib import Path
from PIL import Image, ImageDraw, ImageFilter

out = Path(sys.argv[1])
S = 1024
A = (79, 70, 229)    # indigo-600
B = (6, 182, 212)    # cyan-500


def gradient(size):
    g = Image.new('RGB', (size, size))
    px = g.load()
    for y in range(size):
        for x in range(size):
            t = (x + y) / (2 * (size - 1))
            px[x, y] = tuple(int(A[i] + (B[i] - A[i]) * t) for i in range(3))
    return g


def logo(size, dot=None):
    k = size / 1024
    big = 4  # 超采样抗锯齿
    W = size * big
    kk = k * big
    bg = gradient(W // 8).resize((W, W), Image.BICUBIC)
    mask = Image.new('L', (W, W), 0)
    ImageDraw.Draw(mask).rounded_rectangle([40 * kk, 40 * kk, 984 * kk, 984 * kk], radius=236 * kk, fill=255)
    im = Image.new('RGBA', (W, W), (0, 0, 0, 0))
    im.paste(bg, (0, 0), mask)

    glyph = Image.new('RGBA', (W, W), (0, 0, 0, 0))
    d = ImageDraw.Draw(glyph)
    white = (255, 255, 255, 255)
    w = int(86 * kk)
    # S 形连线：上横 → 右侧半圆下行 → 中横 → 左侧半圆下行 → 下横
    d.line([(340 * kk, 300 * kk), (620 * kk, 300 * kk)], fill=white, width=w)
    d.arc([470 * kk, 300 * kk, 770 * kk, 600 * kk], 270, 90, fill=white, width=w)
    d.line([(620 * kk, 600 * kk), (404 * kk, 600 * kk)], fill=white, width=w)
    d.arc([254 * kk, 600 * kk, 554 * kk, 900 * kk], 90, 270, fill=white, width=w)
    d.line([(404 * kk, 900 * kk), (560 * kk, 900 * kk)], fill=white, width=w)
    # 起点：空心圆（左上）；终点：实心圆（右下）
    r = 120 * kk
    cx, cy = 250 * kk, 300 * kk
    d.ellipse([cx - r, cy - r, cx + r, cy + r], outline=white, width=w)
    r2 = 104 * kk
    cx, cy = 700 * kk, 900 * kk
    d.ellipse([cx - r2, cy - r2, cx + r2, cy + r2], fill=white)
    # 线段端点圆头
    for x, y in ((620, 300), (620, 600), (404, 600), (404, 900)):
        rr = w / 2
        d.ellipse([x * kk - rr, y * kk - rr, x * kk + rr, y * kk + rr], fill=white)

    # 字形整体缩到 74% 并居中（原始坐标 y 范围约 180~1004，中心约 (450, 592)）
    sc = 0.74
    gs = glyph.resize((int(W * sc), int(W * sc)), Image.LANCZOS)
    ox = int(512 * kk - 450 * kk * sc)
    oy = int(512 * kk - 592 * kk * sc)
    im.alpha_composite(gs, (ox, oy))
    d = ImageDraw.Draw(im)
    if dot:
        r = 200 * kk
        cx = cy = 812 * kk
        d.ellipse([cx - r - 48 * kk, cy - r - 48 * kk, cx + r + 48 * kk, cy + r + 48 * kk], fill=(0, 0, 0, 0))
        d.ellipse([cx - r, cy - r, cx + r, cy + r], fill=dot)
    return im.resize((size, size), Image.LANCZOS)


logo(S).save(out / 'app-icon.png')
for name, color in (('ok', (34, 197, 94, 255)), ('warn', (245, 158, 11, 255)), ('err', (239, 68, 68, 255))):
    logo(256, color).resize((64, 64), Image.LANCZOS).save(out / f'tray-{name}.png')
print('ok')
