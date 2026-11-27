#!/bin/sh
# Rewrites change.raw when flag1.png appears, deletes it when flag2.png appears.
T=$1; SH=$2
while [ ! -f "$SH/flag1.png" ]; do sleep 0.1; done
python3 -I -c "
import struct,sys
p=sys.argv[1]; d=bytearray(open(p,'rb').read()); s=d.index(b'Binary:\n')+8
for i in range((len(d)-s)//16): struct.pack_into('<d',d,s+i*16+8,5.0)
open(p,'wb').write(d+d[s:s+16*10])" "$T/change.raw"
while [ ! -f "$SH/flag2.png" ]; do sleep 0.1; done
rm -f "$T/change.raw"
while [ ! -f "$SH/flag3.png" ]; do sleep 0.1; done
rm -f "$3/SAR_top_1GB_tb_tran.uwraw"
