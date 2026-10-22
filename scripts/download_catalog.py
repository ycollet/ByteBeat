#!/usr/bin/env python3
"""Download the public Dollchan catalog without executing its JavaScript."""

import argparse
from concurrent.futures import ThreadPoolExecutor, as_completed
import gzip
import hashlib
import json
import logging
from pathlib import Path
import re
import time
from urllib.error import HTTPError, URLError
from urllib.request import Request, urlopen

BASE = "https://dollchan.net/bytebeat/"
INDEX_URL = BASE + "data/library/all.gz"
LICENSE_URL = "https://raw.githubusercontent.com/SthephanShinkufag/bytebeat-composer/master/LICENSE"


def fetch(url):
    for attempt in range(3):
        try:
            request = Request(url, headers={"User-Agent": "native-bytebeat-catalog/0.1"})
            with urlopen(request, timeout=30) as response:
                return response.read()
        except (HTTPError, URLError, TimeoutError):
            if attempt == 2:
                raise
            time.sleep(2 ** attempt)


def write_json(path, value):
    temporary = path.with_suffix(path.suffix + ".tmp")
    temporary.write_text(json.dumps(value, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    temporary.replace(path)


def download_song(root, author, song, refresh=False):
    song_id = song["hash"]
    if not re.fullmatch(r"[0-9a-f]{32}", song_id):
        raise ValueError(f"Invalid song ID: {song_id!r}")
    path = root / "songs" / f"{song_id}.js"
    # Prefer complete originals, never the truncated codeText preview.
    if "code" in song and not song.get("fileOrig"):
        source_url = INDEX_URL
        content = song["code"].encode("utf-8")
    else:
        kind = next(
            (kind for flag, kind in [
                ("fileOrig", "original"), ("fileMin", "minified"), ("fileForm", "formatted")
            ] if song.get(flag)),
            None,
        )
        if kind is not None:
            source_url = f"{BASE}data/songs/{kind}/{song_id}.js"
            previous = path.with_suffix(".json")
            cached = json.loads(previous.read_text(encoding="utf-8")) if previous.exists() else {}
            content = path.read_bytes() if path.exists() else None
            if (refresh or content is None or cached.get("source") != source_url
                    or cached.get("sha256") != hashlib.sha256(content).hexdigest()
                    or cached.get("upstream") != song):
                content = fetch(source_url)
        elif "codeMin" in song:
            source_url = INDEX_URL
            content = song["codeMin"].encode("utf-8")
        else:
            raise ValueError(f"{song_id}: no complete source available")
    content.decode("utf-8")  # Reject invalid downloads rather than storing unusable scripts.
    metadata = {
        "file": f"songs/{song_id}.js",
        "title": song.get("name") or f"Untitled ({song_id[:8]})",
        "author": song.get("author") or author or "Unknown",
        "mode": song.get("mode", "Bytebeat"),
        "rate": song["sampleRate"],
        "source": source_url,
        "sha256": hashlib.sha256(content).hexdigest(),
        "upstream": song,
    }
    temporary = path.with_suffix(".js.tmp")
    temporary.write_bytes(content)
    temporary.replace(path)
    write_json(path.with_suffix(".json"), metadata)
    return metadata


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, default=Path("data"))
    parser.add_argument("--limit", type=int, help="Download only the first N catalog entries")
    parser.add_argument("--workers", type=int, default=4, choices=range(1, 9))
    parser.add_argument("--refresh", action="store_true", help="Refetch external sources even if cached")
    args = parser.parse_args()
    if args.limit is not None and args.limit < 1:
        parser.error("--limit must be positive")
    logging.basicConfig(level=logging.INFO, format="%(levelname)s %(message)s")
    root = args.output
    (root / "songs").mkdir(parents=True, exist_ok=True)
    archive = fetch(INDEX_URL)
    groups = json.loads(gzip.decompress(archive))
    (root / "all.gz").write_bytes(archive)
    (root / "UPSTREAM-LICENSE").write_bytes(fetch(LICENSE_URL))
    entries = [(group["author"], song) for group in groups for song in group["songs"]]
    if args.limit:
        entries = entries[:args.limit]
    results = {}
    failures = []
    with ThreadPoolExecutor(max_workers=args.workers) as pool:
        jobs = {
            pool.submit(download_song, root, author, song, args.refresh): song
            for author, song in entries
        }
        for done, future in enumerate(as_completed(jobs), 1):
            song = jobs[future]
            try:
                results[song["hash"]] = future.result()
            except (OSError, ValueError, KeyError) as error:
                logging.error("%s: %s", song["hash"], error)
                failures.append({"id": song["hash"], "error": str(error)})
            if done % 100 == 0 or done == len(entries):
                logging.info("%d/%d processed (%d errors)", done, len(entries), len(failures))
    # Keep stable upstream ordering even though downloads run concurrently.
    manifest = {
        "version": 1,
        "source": INDEX_URL,
        "songs": [results[song["hash"]] for _, song in entries if song["hash"] in results],
        "download_errors": failures,
    }
    write_json(root / "catalog.json", manifest)
    logging.info("Saved %d songs to %s/catalog.json", len(results), root)
    return 1 if failures else 0


if __name__ == "__main__":
    raise SystemExit(main())
