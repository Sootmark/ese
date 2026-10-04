#!/bin/sh
# Recreates tests/oracle/*.tsv.gz: libesedb's reading of every test
# database, every table and every value (format in gen.py), compared with
# this crate's by tests/oracle.rs.
#
# libesedb (LGPL) is only run here, never vendored: its Python binding for
# typed values (floats to the bit, text decoded, long values joined), and
# esedbexport for the compressed binary values the binding leaves
# compressed. esedbexport alone isn't enough: it prints floats to six
# decimals and dates as text.
#
# Run on a Linux machine (made with libesedb 20240420 on Debian 13):
#   sudo apt-get install libesedb-utils python3-libesedb
#   sh tests/oracle/gen.sh
set -eu

here=$(cd "$(dirname "$0")" && pwd)
fixtures="$here/../fixtures/plaso"
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

for compressed in "$fixtures"/*.gz; do
    name=$(basename "$compressed" .gz)
    gzip -dc "$compressed" > "$work/$name"
    esedbexport -m tables -t "$work/$name" "$work/$name" > /dev/null
    python3 "$here/gen.py" "$work/$name" "$work/$name.export" > "$work/$name.tsv"
    gzip -9nc "$work/$name.tsv" > "$here/$name.tsv.gz"
done
