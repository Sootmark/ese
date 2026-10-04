#!/bin/sh
# Download SIDR's test Windows.edb (Apache-2.0, a Windows 10 search index,
# 32 MiB, too large to keep here) at a pinned commit, checking its SHA-256,
# into tests/fixtures/sidr/ for tests/sidr.rs. Without it, that test is
# skipped.
set -eu
cd "$(dirname "$0")/fixtures/sidr"
commit=5bde8e87b2f07f72c031eb9bfd39c50739a13aec
sum=10dd5fc05c2d19aa1fa4a705142e413fc5a4af17ae8e5e4909164262f9de7c66
if [ ! -f Windows.edb ]; then
    curl -sfL -o Windows.edb \
        "https://raw.githubusercontent.com/strozfriedberg/sidr/$commit/tests/testdata/Windows.edb"
fi
echo "$sum  Windows.edb" | shasum -a 256 -c --quiet -
