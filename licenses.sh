#!/bin/bash

rm -rf license-list-data
mkdir -p license-list-data/text
mkdir tmp && pushd tmp
git clone --depth=1 https://github.com/spdx/license-list-data
cp license-list-data/text/* ../license-list-data/text/. -v
popd
rm -rf tmp
