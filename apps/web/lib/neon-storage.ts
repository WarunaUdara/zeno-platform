import "server-only";
import { AwsClient } from "aws4fetch";

const BUCKET = "object-upload-download";

function storageClient() {
  const accessKeyId = process.env.AWS_ACCESS_KEY_ID;
  const secretAccessKey = process.env.AWS_SECRET_ACCESS_KEY;
  const endpoint = process.env.AWS_ENDPOINT_URL_S3;
  const region = process.env.AWS_REGION;

  if (!accessKeyId || !secretAccessKey || !endpoint || !region) {
    throw new Error("Neon object storage is not configured");
  }
  if (!["ap-southeast-1", "us-east-1", "us-east-2", "eu-central-1"].includes(region)) {
    throw new Error("Neon object storage region is not supported");
  }

  const parsedEndpoint = new URL(endpoint);
  if (
    parsedEndpoint.protocol !== "https:" ||
    parsedEndpoint.username !== "" ||
    parsedEndpoint.password !== "" ||
    parsedEndpoint.search !== "" ||
    parsedEndpoint.hash !== ""
  ) {
    throw new Error("Neon object storage endpoint must be an HTTPS URL without credentials or query parameters");
  }

  return {
    client: new AwsClient({ accessKeyId, secretAccessKey, service: "s3", region }),
    endpoint: parsedEndpoint,
  };
}

function objectUrl(endpoint: URL, key: string) {
  const url = new URL(endpoint);
  const prefix = url.pathname.replace(/\/$/, "");
  const encodedKey = key.split("/").map(encodeURIComponent).join("/");
  url.pathname = `${prefix}/${BUCKET}/${encodedKey}`;
  return url;
}

export async function putNeonObject(key: string, bytes: Uint8Array, contentType: string) {
  const { client, endpoint } = storageClient();
  const response = await client.fetch(objectUrl(endpoint, key), {
    method: "PUT",
    headers: {
      "content-type": contentType,
      "cache-control": "public, max-age=31536000, immutable",
    },
    body: Buffer.from(bytes),
    signal: AbortSignal.timeout(15_000),
  });
  if (!response.ok) throw new Error("Neon object storage rejected the upload");
}

export async function getNeonObject(key: string) {
  const { client, endpoint } = storageClient();
  return client.fetch(objectUrl(endpoint, key), {
    method: "GET",
    signal: AbortSignal.timeout(10_000),
  });
}

export async function deleteNeonObject(key: string) {
  const { client, endpoint } = storageClient();
  const response = await client.fetch(objectUrl(endpoint, key), {
    method: "DELETE",
    signal: AbortSignal.timeout(10_000),
  });
  if (!response.ok && response.status !== 404) {
    throw new Error("Neon object storage rejected the deletion");
  }
}
