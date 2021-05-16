#!/usr/bin/env bash
# here is Breaking Changes of package in 7.0: https://global.download.synology.com/download/Document/Software/DeveloperGuide/Firmware/DSM/7.0/enu/DSM_Developer_Guide_7_0_Beta.pdf
# https://blog.cmj.tw/SynologyApp.htm 暂时不搞签名
/bin/rm -rf package
mkdir package
cd package
mkdir bin logs config
echo port=21116 > config/hbbs.conf
echo key= >> config/hbbs.conf
echo email= >> config/hbbs.conf
echo port=21117 > config/hbbr.conf
cp ../target/release/hbbs bin/
cp ../target/release/hbbr bin/
strip bin/hbbs
strip bin/hbbr
tar -czf ../spk/package.tgz *
cd ..
cd spk
#md5 package.tgz | awk '{print "checksum=\"" $4 "\""}' >> INFO
file=rustdesk-server-synology-x64.spk
tar -cvf $file *
mv $file ..
