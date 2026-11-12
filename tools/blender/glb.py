"""Minimal GLB reader/writer used by the enemy builder (no third-party glTF library needed)."""
import json
import struct

import numpy as np

COMP = {5120: np.int8, 5121: np.uint8, 5122: np.int16, 5123: np.uint16, 5125: np.uint32, 5126: np.float32}
NCOMP = {"SCALAR": 1, "VEC2": 2, "VEC3": 3, "VEC4": 4, "MAT4": 16}


class Glb:
    def __init__(self, path):
        b = open(path, "rb").read()
        n = struct.unpack("<I", b[12:16])[0]
        self.j = json.loads(b[20:20 + n])
        off = 20 + n
        bl = struct.unpack("<I", b[off:off + 4])[0]
        self.bin = bytearray(b[off + 8:off + 8 + bl])

    def acc(self, i):
        a = self.j["accessors"][i]
        bv = self.j["bufferViews"][a["bufferView"]]
        dt = COMP[a["componentType"]]
        nc = NCOMP[a["type"]]
        start = bv.get("byteOffset", 0) + a.get("byteOffset", 0)
        stride = bv.get("byteStride", 0)
        isz = np.dtype(dt).itemsize * nc
        if stride and stride != isz:
            raw = np.frombuffer(bytes(self.bin), dtype=np.uint8)
            rows = [raw[start + k * stride:start + k * stride + isz] for k in range(a["count"])]
            arr = np.frombuffer(b"".join(r.tobytes() for r in rows), dtype=dt)
        else:
            arr = np.frombuffer(bytes(self.bin[start:start + isz * a["count"]]), dtype=dt)
        arr = arr.reshape(a["count"], nc) if nc > 1 else arr.copy()
        if a.get("normalized") and dt != np.float32:
            arr = arr.astype(np.float32) / np.iinfo(dt).max
        return np.array(arr)

    def add(self, arr, kind, comp, target=None, minmax=False):
        """Append an accessor (and its own buffer view) holding `arr`; returns its index."""
        data = np.ascontiguousarray(arr.astype(COMP[comp])).tobytes()
        while len(self.bin) % 4:
            self.bin.append(0)
        bv = {"buffer": 0, "byteOffset": len(self.bin), "byteLength": len(data)}
        if target:
            bv["target"] = target
        self.bin.extend(data)
        self.j["bufferViews"].append(bv)
        a = {"bufferView": len(self.j["bufferViews"]) - 1, "componentType": comp, "count": int(arr.shape[0]), "type": {1: "SCALAR", 2: "VEC2", 3: "VEC3", 4: "VEC4"}[1 if arr.ndim == 1 else arr.shape[1]]}
        if comp in (5121, 5123) and kind == "color":
            a["normalized"] = True
        if minmax:
            a["min"] = [float(x) for x in arr.min(axis=0)]
            a["max"] = [float(x) for x in arr.max(axis=0)]
        self.j["accessors"].append(a)
        return len(self.j["accessors"]) - 1

    def compact(self):
        """Drop accessors and buffer views nothing references any more (the replaced vertex data)."""
        j = self.j
        used = []

        def keep(i):
            if i not in used:
                used.append(i)

        for m in j.get("meshes", []):
            for p in m["primitives"]:
                for a in p["attributes"].values():
                    keep(a)
                if "indices" in p:
                    keep(p["indices"])
        for sk in j.get("skins", []):
            if "inverseBindMatrices" in sk:
                keep(sk["inverseBindMatrices"])
        for an in j.get("animations", []):
            for sm in an["samplers"]:
                keep(sm["input"])
                keep(sm["output"])
        amap = {old: new for new, old in enumerate(used)}
        accs = [dict(j["accessors"][i]) for i in used]
        views, vmap, data = [], {}, bytearray()
        for a in accs:
            v = a["bufferView"]
            if v not in vmap:
                bv = dict(j["bufferViews"][v])
                chunk = self.bin[bv.get("byteOffset", 0):bv.get("byteOffset", 0) + bv["byteLength"]]
                while len(data) % 4:
                    data.append(0)
                bv["byteOffset"] = len(data)
                data.extend(chunk)
                vmap[v] = len(views)
                views.append(bv)
            a["bufferView"] = vmap[a["bufferView"]]
        for m in j.get("meshes", []):
            for p in m["primitives"]:
                p["attributes"] = {k: amap[v] for k, v in p["attributes"].items()}
                if "indices" in p:
                    p["indices"] = amap[p["indices"]]
        for sk in j.get("skins", []):
            if "inverseBindMatrices" in sk:
                sk["inverseBindMatrices"] = amap[sk["inverseBindMatrices"]]
        for an in j.get("animations", []):
            for sm in an["samplers"]:
                sm["input"] = amap[sm["input"]]
                sm["output"] = amap[sm["output"]]
        j["accessors"] = accs
        j["bufferViews"] = views
        self.bin = data

    def save(self, path):
        self.compact()
        self.j["buffers"][0]["byteLength"] = len(self.bin)
        js = json.dumps(self.j, separators=(",", ":")).encode()
        while len(js) % 4:
            js += b" "
        while len(self.bin) % 4:
            self.bin.append(0)
        total = 12 + 8 + len(js) + 8 + len(self.bin)
        with open(path, "wb") as f:
            f.write(struct.pack("<III", 0x46546C67, 2, total))
            f.write(struct.pack("<II", len(js), 0x4E4F534A))
            f.write(js)
            f.write(struct.pack("<II", len(self.bin), 0x004E4942))
            f.write(self.bin)

    # ---- skeleton helpers
    def skin(self):
        return self.j["skins"][0]

    def joint_names(self):
        return [self.j["nodes"][n].get("name", "") for n in self.skin()["joints"]]

    def bind_positions(self):
        """Bind-pose position of every joint in mesh space (inverse of the inverse bind matrix)."""
        ibm = self.acc(self.skin()["inverseBindMatrices"]).reshape(-1, 4, 4)
        out = {}
        for name, m in zip(self.joint_names(), ibm):
            w = np.linalg.inv(m.T)  # glTF matrices are column-major
            out[name] = w[:3, 3].copy()
        return out

    def bind_matrices(self):
        ibm = self.acc(self.skin()["inverseBindMatrices"]).reshape(-1, 4, 4)
        return {name: np.linalg.inv(m.T) for name, m in zip(self.joint_names(), ibm)}

    def skinned_prims(self):
        """(mesh index, primitive) for every primitive of a skinned node."""
        out = []
        for nd in self.j["nodes"]:
            if "skin" in nd and "mesh" in nd:
                for p in self.j["meshes"][nd["mesh"]]["primitives"]:
                    out.append((nd["mesh"], p))
        return out
