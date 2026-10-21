import sharp from "sharp";

const MAX_COVER_IMAGE_PIXELS = 40_000_000;

export async function normalizeCoverImage(bytes: Uint8Array): Promise<Buffer> {
  return sharp(Buffer.from(bytes), {
    failOn: "warning",
    limitInputPixels: MAX_COVER_IMAGE_PIXELS,
  })
    .rotate()
    .resize({ width: 2400, height: 1350, fit: "inside", withoutEnlargement: true })
    .webp({ quality: 82 })
    .toBuffer();
}
