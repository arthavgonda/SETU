#!/usr/bin/env python3
import argparse
import os
import shutil
import subprocess
import sys

AVATAR = 360
BACKGROUND = (1280, 720)
THUMB = (256, 144)


def pngs(d):
    return sorted(f for f in os.listdir(d) if f.lower().endswith(".png")) if os.path.isdir(d) else []


def square(img):
    w, h = img.size
    if h > w:
        top = int((h - w) * 0.2)
        return img.crop((0, top, w, top + w))
    left = (w - h) // 2
    return img.crop((left, 0, left + h, h))


def cover(img, size):
    w, h = img.size
    tw, th = size
    if w * th > h * tw:
        nw = h * tw // th
        left = (w - nw) // 2
        img = img.crop((left, 0, left + nw, h))
    else:
        nh = w * th // tw
        top = (h - nh) // 2
        img = img.crop((0, top, w, top + nh))
    if img.size[0] > tw:
        img = img.resize(size, LANCZOS)
    return img


def with_pillow(src, out):
    from PIL import Image, ImageDraw
    global LANCZOS
    LANCZOS = Image.Resampling.LANCZOS
    for f in pngs(os.path.join(src, "avatars")):
        img = square(Image.open(os.path.join(src, "avatars", f)).convert("RGBA")).resize((AVATAR, AVATAR), LANCZOS)
        big = Image.new("L", (AVATAR * 4, AVATAR * 4), 0)
        ImageDraw.Draw(big).ellipse((0, 0, AVATAR * 4 - 1, AVATAR * 4 - 1), fill=255)
        mask = big.resize((AVATAR, AVATAR), LANCZOS)
        alpha = Image.composite(img.getchannel("A"), Image.new("L", img.size, 0), mask)
        img.putalpha(alpha)
        img.save(os.path.join(out, "avatars", f), optimize=True)
    for f in pngs(os.path.join(src, "avatars_bg")):
        img = Image.open(os.path.join(src, "avatars_bg", f)).convert("RGB")
        stem = os.path.splitext(f)[0]
        cover(img, BACKGROUND).save(os.path.join(out, "backgrounds", stem + ".jpg"), quality=84, optimize=True, progressive=True)
        cover(img, THUMB).save(os.path.join(out, "thumbs", stem + ".jpg"), quality=80, optimize=True)


def with_magick(src, out, tool):
    for f in pngs(os.path.join(src, "avatars")):
        s = os.path.join(src, "avatars", f)
        subprocess.run(tool + [s, "-alpha", "on", "-gravity", "north", "-resize", "%dx%d^" % (AVATAR, AVATAR), "-extent", "%dx%d" % (AVATAR, AVATAR), "(", "+clone", "-alpha", "extract", "-fill", "black", "-colorize", "100", "-fill", "white", "-draw", "circle %d,%d %d,0" % (AVATAR // 2, AVATAR // 2, AVATAR // 2), ")", "-compose", "CopyOpacity", "-composite", os.path.join(out, "avatars", f)], check=True)
    for f in pngs(os.path.join(src, "avatars_bg")):
        s = os.path.join(src, "avatars_bg", f)
        stem = os.path.splitext(f)[0]
        for size, d, q in ((BACKGROUND, "backgrounds", 84), (THUMB, "thumbs", 80)):
            subprocess.run(tool + [s, "-gravity", "center", "-resize", "%dx%d^>" % size, "-extent", "%dx%d" % size, "-quality", str(q), os.path.join(out, d, stem + ".jpg")], check=True)


def copied(src, out):
    for f in pngs(os.path.join(src, "avatars")):
        shutil.copyfile(os.path.join(src, "avatars", f), os.path.join(out, "avatars", f))
    for f in pngs(os.path.join(src, "avatars_bg")):
        stem = os.path.splitext(f)[0]
        for d in ("backgrounds", "thumbs"):
            shutil.copyfile(os.path.join(src, "avatars_bg", f), os.path.join(out, d, stem + ".jpg"))


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--src", required=True)
    ap.add_argument("--out", required=True)
    a = ap.parse_args()
    for d in ("avatars", "backgrounds", "thumbs"):
        os.makedirs(os.path.join(a.out, d), exist_ok=True)
    try:
        import PIL
        with_pillow(a.src, a.out)
        return
    except ImportError:
        pass
    tool = [shutil.which("magick")] if shutil.which("magick") else ([shutil.which("convert")] if shutil.which("convert") else None)
    if tool:
        with_magick(a.src, a.out, tool)
        return
    print("scale-looks: neither Pillow nor ImageMagick was found, so the profile pictures are packed at full size", file=sys.stderr)
    copied(a.src, a.out)


if __name__ == "__main__":
    main()
