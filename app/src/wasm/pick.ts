// Lists files the user picked or dropped. Nothing is read until `read()` is
// called, so callers can filter by path first.
import { wantsDir } from "./files";

export type Picked = { path: string; read: () => Promise<ArrayBuffer> };

function readBatch(reader: FileSystemDirectoryReader): Promise<FileSystemEntry[]> {
  return new Promise((resolve, reject) => reader.readEntries(resolve, reject));
}

async function walk(entry: FileSystemEntry, out: Picked[], top: boolean): Promise<void> {
  if (entry.isFile) {
    const fe = entry as FileSystemFileEntry;
    out.push({
      path: entry.fullPath.replace(/^\//, ""),
      read: async () => (await new Promise<File>((resolve, reject) => fe.file(resolve, reject))).arrayBuffer(),
    });
  } else if (entry.isDirectory && (top || wantsDir(entry.name))) {
    const reader = (entry as FileSystemDirectoryEntry).createReader();
    // readEntries returns batches; an empty batch means done.
    for (let batch = await readBatch(reader); batch.length > 0; batch = await readBatch(reader)) {
      for (const child of batch) await walk(child, out, false);
    }
  }
}

/** Files from a dropped DataTransfer (folders are walked recursively, skipping dot-folders). */
export async function fromDrop(dt: DataTransfer): Promise<Picked[]> {
  // webkitGetAsEntry must be called synchronously, before any await.
  const entries = [...dt.items].map((i) => i.webkitGetAsEntry()).filter((e): e is FileSystemEntry => e !== null);
  const out: Picked[] = [];
  for (const entry of entries) await walk(entry, out, true);
  return out;
}

/** Files from an <input type=file>, using webkitRelativePath when present. */
export function fromInput(files: FileList): Picked[] {
  return [...files].map((f) => ({ path: f.webkitRelativePath || f.name, read: () => f.arrayBuffer() }));
}

export const isZip = (files: Picked[]) => files.length === 1 && /\.zip$/i.test(files[0].path);
