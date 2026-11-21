#!/usr/bin/env python3
"""Decode the self-contained reference archive without loading its cell tree.

The v1 envelope contains gzip/base64 chunks of the exact original JSON bytes,
including every historical generation. Plain legacy JSON is also readable.
No external files, libraries, URLs, or source checkout are needed for decoding.
"""
import argparse
import base64
import hashlib
import gzip
import re
import json
import os
from pathlib import Path
import tempfile
import zlib

FORMAT = "antislop-reference-archive-v1"


def unique_object(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError(f"duplicate JSON key: {key}")
        result[key] = value
    return result


def validate(archive):
    required = {"format", "encoding", "decoded_format", "decoded_bytes", "decoded_sha256", "payload"}
    if not isinstance(archive, dict) or set(archive) != required or archive["format"] != FORMAT or archive["encoding"] != "gzip+base64":
        raise ValueError("unsupported reference archive schema or encoding")
    if (type(archive["decoded_bytes"]) is not int or archive["decoded_bytes"] < 0
            or not isinstance(archive["decoded_sha256"], str)
            or not re.fullmatch(r"[0-9a-f]{64}", archive["decoded_sha256"])
            or not isinstance(archive["decoded_format"], str)
            or not isinstance(archive["payload"], list)
            or not archive["payload"]
            or not all(isinstance(chunk, str) for chunk in archive["payload"])):
        raise ValueError("invalid reference archive field types")


def pack(source, destination):
    """Stream exact input bytes into a self-contained, versioned JSON archive."""
    source, destination = Path(source), Path(destination)
    digest, count = hashlib.sha256(), 0
    with tempfile.TemporaryFile(dir=destination.parent) as compressed:
        with gzip.GzipFile(fileobj=compressed, mode="wb", filename="", mtime=0) as encoder:
            with source.open("rb") as stream:
                while chunk := stream.read(1024 * 1024):
                    digest.update(chunk)
                    count += len(chunk)
                    encoder.write(chunk)
        compressed.seek(0)
        metadata = dict(format=FORMAT, encoding="gzip+base64",
                        decoded_format="original complete reference-renders JSON, byte for byte",
                        decoded_bytes=count, decoded_sha256=digest.hexdigest())
        with tempfile.NamedTemporaryFile(mode="w", encoding="utf-8", dir=destination.parent, delete=False) as output:
            temporary = Path(output.name)
            try:
                output.write(json.dumps(metadata, indent=2)[:-1] + ',\n  "payload": [\n')
                first = True
                while chunk := compressed.read(49152):
                    if not first:
                        output.write(',\n')
                    output.write('    ' + json.dumps(base64.b64encode(chunk).decode('ascii')))
                    first = False
                output.write('\n  ]\n}\n')
                output.flush()
                os.fsync(output.fileno())
                os.link(temporary, destination)
            finally:
                temporary.unlink(missing_ok=True)
    return metadata


def decode(source, destination):
    """Publish a new decoded file only after length, SHA-256 and gzip checks."""
    source, destination = Path(source), Path(destination)
    # Only the compressed envelope is materialized. Legacy cells are streamed.
    with source.open("rb") as stream:
        prefix = stream.read(256)
    envelope = b'"antislop-reference-archive-' in prefix or b'"gzip+base64"' in prefix
    archive = None
    if envelope:
        with source.open(encoding="utf-8") as stream:
            archive = json.load(stream, object_pairs_hook=unique_object)
        validate(archive)
    digest = hashlib.sha256()
    count = 0
    with tempfile.NamedTemporaryFile(dir=destination.parent, delete=False) as output:
        temporary = Path(output.name)
        try:
            def write(chunk):
                nonlocal count
                output.write(chunk)
                digest.update(chunk)
                count += len(chunk)

            if archive is None:
                with source.open("rb") as stream:
                    while chunk := stream.read(1024 * 1024):
                        write(chunk)
            else:
                decoder = zlib.decompressobj(31)
                for chunk in archive["payload"]:
                    write(decoder.decompress(base64.b64decode(chunk, validate=True)))
                write(decoder.flush())
                if not decoder.eof or decoder.unused_data or decoder.unconsumed_tail:
                    raise ValueError("incomplete or trailing gzip reference data")
                if count != archive["decoded_bytes"] or digest.hexdigest() != archive["decoded_sha256"]:
                    raise ValueError("reference archive length or SHA-256 mismatch")
            output.flush()
            os.fsync(output.fileno())
            # Hard-link publication refuses to replace an existing result.
            os.link(temporary, destination)
        finally:
            temporary.unlink(missing_ok=True)
    return {"decoded_bytes": count, "decoded_sha256": digest.hexdigest()}


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--pack", action="store_true", help="encode decoded JSON instead of decoding")
    parser.add_argument("source", type=Path)
    parser.add_argument("destination", type=Path, help="new output file; never overwritten")
    args = parser.parse_args()
    print(json.dumps((pack if args.pack else decode)(args.source, args.destination)))
