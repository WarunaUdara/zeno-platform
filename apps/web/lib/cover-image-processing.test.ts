import assert from "node:assert/strict";
import { describe, it } from "node:test";
import sharp from "sharp";
import { normalizeCoverImage } from "./cover-image-processing";

describe("cover image processing", () => {
  it("normalizes accepted raster images into small metadata-free WebP", async () => {
    const input = await sharp({
      create: { width: 12, height: 8, channels: 3, background: "#d21c3c" },
    })
      .png()
      .toBuffer();
    const normalized = await normalizeCoverImage(input);
    const metadata = await sharp(normalized).metadata();

    assert.equal(metadata.format, "webp");
    assert.equal(metadata.width, 12);
    assert.equal(metadata.height, 8);
    assert.equal(metadata.exif, undefined);
  });
});
