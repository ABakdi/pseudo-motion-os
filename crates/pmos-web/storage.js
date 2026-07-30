// VFS persistence (Architecture §3): the kernel's in-memory tree mirrored
// into the Origin Private File System. Boot loads everything into the
// kernel; afterwards the platform drains write-through ops here.
(() => {
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
      window.wasmBindings.pmos_vfs_ready(true, "");
    } catch (err) {
      console.warn("[pmos storage] OPFS unavailable:", err);
      window.wasmBindings.pmos_vfs_ready(false, String(err));
    }
  }

  async function write(path, data) {
    try {
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
      const { dir, file } = await dirFor(path, false);
      await dir.removeEntry(file, { recursive: true });
    } catch (err) {
      // Deleting something already gone is fine.
    }
  }

  async function mkdir(path) {
    try {
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
