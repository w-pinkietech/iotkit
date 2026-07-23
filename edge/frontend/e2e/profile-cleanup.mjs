import { rm } from "node:fs/promises";

export async function removeChromiumProfile(profile, remove = rm) {
  await remove(profile, {
    recursive: true,
    force: true,
    maxRetries: 5,
    retryDelay: 100,
  });
}
