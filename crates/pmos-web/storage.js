// VFS persistence (Architecture §3): the kernel's in-memory tree mirrored
// into the Origin Private File System. Boot loads everything into the
// kernel; afterwards the platform drains write-through ops here.
//
// Fallback (M6 deferral, shipped 2026-08-04): browsers/contexts without a
// working OPFS (some private windows, older engines) get the same tree in
// IndexedDB — two flat stores, "files" (path → bytes) and "dirs" (path
// markers). The kernel never knows which backend is under it.
(() => {
  let backend = null; // "opfs" | "idb" — chosen once, at load
  let rootPromise = null;
  const root = () => (rootPromise ??= navigator.storage.getDirectory());

  async function dirFor(path, create) {
    // path like /notes/inbox/x.md → walk to the parent directory handle.
    const parts = path.split("/").filter(Boolean);
    const file = parts.pop();
    let dir = await root();
    for (const part of parts) {
      dir = await dir.getDirectoryHandle(part, { create });
    }
    return { dir, file };
  }

  // ---- IndexedDB backend ----
  let dbPromise = null;
  const idb = () =>
    (dbPromise ??= new Promise((resolve, reject) => {
      const req = indexedDB.open("pmos-vfs", 1);
      req.onupgradeneeded = () => {
        req.result.createObjectStore("files");
        req.result.createObjectStore("dirs");
      };
      req.onsuccess = () => resolve(req.result);
      req.onerror = () => reject(req.error);
    }));
  const wait = (r) =>
    new Promise((resolve, reject) => {
      r.onsuccess = () => resolve(r.result);
      r.onerror = () => reject(r.error);
    });

  async function idbLoadAll() {
    const db = await idb();
    const tx = db.transaction(["dirs", "files"], "readonly");
    for (const d of await wait(tx.objectStore("dirs").getAllKeys())) {
      window.wasmBindings.pmos_vfs_dir(d);
    }
    const files = tx.objectStore("files");
    const keys = await wait(files.getAllKeys());
    const vals = await wait(files.getAll());
    keys.forEach((k, i) =>
      window.wasmBindings.pmos_vfs_file(k, new Uint8Array(vals[i]))
    );
  }

  async function idbWrite(path, data) {
    const db = await idb();
    await wait(
      db.transaction("files", "readwrite").objectStore("files").put(data, path)
    );
  }

  async function idbRemove(path) {
    // Matches OPFS removeEntry({recursive:true}): the path and any children.
    const db = await idb();
    const prefix = path + "/";
    for (const store of ["files", "dirs"]) {
      const tx = db.transaction(store, "readwrite").objectStore(store);
      for (const k of await wait(tx.getAllKeys())) {
        if (k === path || k.startsWith(prefix)) await wait(tx.delete(k));
      }
    }
  }

  async function idbMkdir(path) {
    const db = await idb();
    await wait(
      db.transaction("dirs", "readwrite").objectStore("dirs").put(1, path)
    );
  }

  // ---- boot: pick a backend, load everything ----
  async function loadAll() {
    const out = [];
    async function walk(dir, prefix) {
      for await (const [name, handle] of dir.entries()) {
        const path = `${prefix}/${name}`;
        if (handle.kind === "directory") {
          window.wasmBindings.pmos_vfs_dir(path);
          await walk(handle, path);
        } else {
          const data = new Uint8Array(await (await handle.getFile()).arrayBuffer());
          out.push([path, data]);
        }
      }
    }
    try {
      await walk(await root(), "");
      for (const [path, data] of out) {
        window.wasmBindings.pmos_vfs_file(path, data);
      }
      backend = "opfs";
      window.wasmBindings.pmos_vfs_ready(true, "");
      return;
    } catch (err) {
      console.warn("[pmos storage] OPFS unavailable, trying IndexedDB:", err);
    }
    try {
      await idbLoadAll();
      backend = "idb";
      console.log("[pmos storage] persisting via IndexedDB fallback");
      window.wasmBindings.pmos_vfs_ready(true, "");
    } catch (err) {
      console.warn("[pmos storage] IndexedDB unavailable too:", err);
      window.wasmBindings.pmos_vfs_ready(false, String(err));
    }
  }

  async function write(path, data) {
    try {
      if (backend === "idb") return await idbWrite(path, data);
      const { dir, file } = await dirFor(path, true);
      const handle = await dir.getFileHandle(file, { create: true });
      const w = await handle.createWritable();
      await w.write(data);
      await w.close();
    } catch (err) {
      console.error("[pmos storage] write failed", path, err);
    }
  }

  async function remove(path) {
    try {
      if (backend === "idb") return await idbRemove(path);
      const { dir, file } = await dirFor(path, false);
      await dir.removeEntry(file, { recursive: true });
    } catch (err) {
      // Deleting something already gone is fine.
    }
  }

  async function mkdir(path) {
    try {
      if (backend === "idb") return await idbMkdir(path);
      const parts = path.split("/").filter(Boolean);
      let dir = await root();
      for (const part of parts) {
        dir = await dir.getDirectoryHandle(part, { create: true });
      }
    } catch (err) {
      console.error("[pmos storage] mkdir failed", path, err);
    }
  }

  window.pmosStorage = { loadAll, write, remove, mkdir };
})();
