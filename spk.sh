#!/usr/bin/env bash
/bin/rm -rf package
mkdir package
cd package
mkdir bin logs config
echo port=21116 > config/hbbs.conf
echo key= >> config/hbbs.conf
echo port=21117 > config/hbbr.conf
echo key= >> config/hbbr.conf
cp ../target/release/hbbs bin/
cp ../target/release/hbbr bin/
strip bin/hbbs
strip bin/hbbr
tar czf ../spk/package.tgz ./*
cd ..
cd spk
VER=1.1.3
tar cf RustDeskServer-x64-$VER.spk ./*
mv RustDeskServer-x64-$VER.spk ..
