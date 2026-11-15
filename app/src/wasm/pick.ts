// Reads files the user picked or dropped in the page into path + bytes pairs.
export type Picked = { path: string; data: ArrayBuffer };

async function readFile(file: File, path: string): Promise<Picked> {
  return { path, data: await file.arrayBuffer() };
}

function readBatch(reader: FileSystemDirectoryReader): Promise<FileSystemEntry[]> {
  return new Promise((resolve, reject) => reader.readEntries(resolve, reject));
}

async function walk(entry: FileSystemEntry, out: Picked[]): Promise<void> {
  if (entry.isFile) {
    const file = await new Promise<File>((resolve, reject) => (entry as FileSystemFileEntry).file(resolve, reject));
    out.push(await readFile(file, entry.fullPath.replace(/^\//, "")));
  } else if (entry.isDirectory) {
    const reader = (entry as FileSystemDirectoryEntry).createReader();
    // readEntries returns batches; an empty batch means done.
    for (let batch = await readBatch(reader); batch.length > 0; batch = await readBatch(reader)) {
      for (const child of batch) await walk(child, out);
    }
  }
}

/** Files from a dropped DataTransfer (folders are walked recursively). */
export async function fromDrop(dt: DataTransfer): Promise<Picked[]> {
  // webkitGetAsEntry must be called synchronously, before any await.
  const entries = [...dt.items].map((i) => i.webkitGetAsEntry()).filter((e): e is FileSystemEntry => e !== null);
  const out: Picked[] = [];
  for (const entry of entries) await walk(entry, out);
  return out;
}

/** Files from an <input type=file>, using webkitRelativePath when present. */
export async function fromInput(files: FileList): Promise<Picked[]> {
  return Promise.all([...files].map((f) => readFile(f, f.webkitRelativePath || f.name)));
}

export const isZip = (files: Picked[]) => files.length === 1 && /\.zip$/i.test(files[0].path);
