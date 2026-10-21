import assert from "node:assert/strict";
import { describe, it } from "node:test";
import { detectCoverImageContentType, MAX_COVER_IMAGE_BYTES } from "./cover-image-validation";

describe("cover image upload validation", () => {
  it("recognizes supported image signatures", () => {
    assert.equal(detectCoverImageContentType(Uint8Array.from([0xff, 0xd8, 0xff, 0x00])), "image/jpeg");
    assert.equal(detectCoverImageContentType(Uint8Array.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a])), "image/png");
    assert.equal(detectCoverImageContentType(Uint8Array.from([0x52, 0x49, 0x46, 0x46, 0, 0, 0, 0, 0x57, 0x45, 0x42, 0x50])), "image/webp");
  });

  it("rejects formats that must not be served as event artwork", () => {
    assert.equal(detectCoverImageContentType(new TextEncoder().encode("<svg></svg>")), null);
  });

  it("caps uploads at five mebibytes", () => {
    assert.equal(MAX_COVER_IMAGE_BYTES, 5 * 1024 * 1024);
  });
});
