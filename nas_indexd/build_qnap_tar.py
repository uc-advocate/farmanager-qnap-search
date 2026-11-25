#!/usr/bin/env python3
"""Repack a Docker 25+ OCI `docker save` archive into classic Docker v1.2 format
(manifest.json + repositories + <layer>/layer.tar) for compatibility with older
QNAP Container Station / Docker daemons.

Usage:
    docker build -t nas-indexd:latest .
    docker save -o nas-indexd.tar nas-indexd:latest
    python3 build_qnap_tar.py nas-indexd.tar nas-indexd-qnap.tar
"""

import io
import json
import sys
import tarfile


def repack(src_path: str, dst_path: str, repo: str = "nas-indexd", tag: str = "latest") -> None:
    with tarfile.open(src_path, "r") as src:
        manifest = json.loads(src.extractfile("manifest.json").read())
        entry = manifest[0]
        cfg_src = entry["Config"]
        cfg_bytes = src.extractfile(cfg_src).read()
        cfg_hash = cfg_src.split("/")[-1]
        cfg_name = cfg_hash + ".json"

        new_layers = []
        with tarfile.open(dst_path, "w") as dst:
            ti = tarfile.TarInfo(cfg_name)
            ti.size = len(cfg_bytes)
            dst.addfile(ti, io.BytesIO(cfg_bytes))

            parent = ""
            last_layer_id = ""
            for layer_src in entry["Layers"]:
                lbytes = src.extractfile(layer_src).read()
                lhash = layer_src.split("/")[-1]
                last_layer_id = lhash

                vbytes = b"1.0"
                tiv = tarfile.TarInfo(f"{lhash}/VERSION")
                tiv.size = len(vbytes)
                dst.addfile(tiv, io.BytesIO(vbytes))

                ljson = {"id": lhash}
                if parent:
                    ljson["parent"] = parent
                jbytes = json.dumps(ljson).encode()
                tij = tarfile.TarInfo(f"{lhash}/json")
                tij.size = len(jbytes)
                dst.addfile(tij, io.BytesIO(jbytes))

                til = tarfile.TarInfo(f"{lhash}/layer.tar")
                til.size = len(lbytes)
                dst.addfile(til, io.BytesIO(lbytes))

                new_layers.append(f"{lhash}/layer.tar")
                parent = lhash

            new_manifest = [{
                "Config": cfg_name,
                "RepoTags": [f"{repo}:{tag}"],
                "Layers": new_layers,
            }]
            mbytes = json.dumps(new_manifest).encode()
            tim = tarfile.TarInfo("manifest.json")
            tim.size = len(mbytes)
            dst.addfile(tim, io.BytesIO(mbytes))

            rbytes = json.dumps({repo: {tag: last_layer_id}}).encode()
            tir = tarfile.TarInfo("repositories")
            tir.size = len(rbytes)
            dst.addfile(tir, io.BytesIO(rbytes))


if __name__ == "__main__":
    src = sys.argv[1] if len(sys.argv) > 1 else "nas-indexd.tar"
    dst = sys.argv[2] if len(sys.argv) > 2 else "nas-indexd-qnap.tar"
    repack(src, dst)
    print(f"Created classic Docker v1.2 archive: {dst}")
