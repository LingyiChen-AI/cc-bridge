"""生成安装程序用的图片：NSIS 顶部横幅 150x57、侧边图 164x314，WiX 横幅 493x58、对话框背景 493x312（都是 24 位 BMP）。
logo 取 src-tauri/icons/app-icon.png，风格和应用一致：黑底、白字、陶土橙点缀。"""
from pathlib import Path
from PIL import Image, ImageDraw, ImageFont

root = Path(__file__).resolve().parent.parent
icons = root / 'src-tauri' / 'icons'
out = icons / 'installer'
out.mkdir(exist_ok=True)
logo = Image.open(icons / 'app-icon.png').convert('RGBA')
BG = (20, 20, 20)
WHITE = (255, 255, 255)
GREY = (150, 150, 150)
ORANGE = (217, 119, 87)


def font(size, bold=True):
    for name in (('segoeuib.ttf' if bold else 'segoeui.ttf'), 'arialbd.ttf', 'arial.ttf'):
        try:
            return ImageFont.truetype(name, size)
        except OSError:
            pass
    return ImageFont.load_default()


def paste_logo(im, size, xy):
    im.paste(logo.resize((size, size), Image.LANCZOS), xy, logo.resize((size, size), Image.LANCZOS))


def save(im, name):
    im.convert('RGB').save(out / name, 'BMP')


# NSIS 侧边图（欢迎页 / 完成页左侧）
s = 2
im = Image.new('RGBA', (164 * s, 314 * s), BG + (255,))
d = ImageDraw.Draw(im)
paste_logo(im, 72 * s, (24 * s, 40 * s))
d.text((24 * s, 132 * s), 'CC Bridge', font=font(22 * s), fill=WHITE)
d.text((24 * s, 162 * s), 'Codex  >  Claude Code', font=font(11 * s, False), fill=GREY)
d.rectangle([24 * s, 290 * s, 48 * s, 293 * s], fill=ORANGE)
save(im.resize((164, 314), Image.LANCZOS), 'nsis-sidebar.bmp')

# NSIS 顶部横幅（白底，右侧 logo）
im = Image.new('RGBA', (150 * s, 57 * s), (255, 255, 255, 255))
paste_logo(im, 40 * s, (100 * s, 8 * s))
save(im.resize((150, 57), Image.LANCZOS), 'nsis-header.bmp')

# WiX 横幅（白底，右侧 logo）
im = Image.new('RGBA', (493 * s, 58 * s), (255, 255, 255, 255))
paste_logo(im, 42 * s, (440 * s, 8 * s))
save(im.resize((493, 58), Image.LANCZOS), 'wix-banner.bmp')

# WiX 对话框背景：左侧 164 宽为品牌区，右侧留白放文字
im = Image.new('RGBA', (493 * s, 312 * s), (255, 255, 255, 255))
d = ImageDraw.Draw(im)
d.rectangle([0, 0, 164 * s, 312 * s], fill=BG)
paste_logo(im, 72 * s, (24 * s, 40 * s))
d.text((24 * s, 132 * s), 'CC Bridge', font=font(22 * s), fill=WHITE)
d.text((24 * s, 162 * s), 'Codex  >  Claude Code', font=font(11 * s, False), fill=GREY)
save(im.resize((493, 312), Image.LANCZOS), 'wix-dialog.bmp')
print('ok', out)
