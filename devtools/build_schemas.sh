#!/bin/bash
set -o errexit -o nounset -o pipefail
command -v shellcheck >/dev/null && shellcheck "$0"

rm -rf ./schemas
mkdir -p ./schemas

BASEDIR=$(pwd)
for C in contracts/*/Cargo.toml contracts/consumer/*/Cargo.toml; do
  DIR=$(dirname "$C")
  echo "Building schema for $DIR"
  (
    cd "$DIR"; 
    cargo schema > /dev/null;
    ls ./schema/*.json;
    cp ./schema/*.json "$BASEDIR/schemas";
    cd -
  )
done
