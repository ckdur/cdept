"""Corrupt good.uwraw's chunk table (its source stamps stay fresh) and rewrite
stale.raw with other values of the same size. Usage: python3 -I corrupt.py DIR"""
import os, struct, sys, time
d = sys.argv[1]
p = os.path.join(d, "good.uwraw")
b = bytearray(open(p, "rb").read())
chunk_table = struct.unpack_from("<Q", b, 32)[0]
struct.pack_into("<Q", b, chunk_table + 4, len(b) * 4)   # first chunk points past the end
open(p, "wb").write(b)
s = os.path.join(d, "stale.raw")
data = bytearray(open(s, "rb").read())
start = data.index(b"Binary:\n") + 8
n = (len(data) - start) // 16
for i in range(n):
    struct.pack_into("<d", data, start + i * 16 + 8, 3.0)
time.sleep(0.05)
open(s, "wb").write(data)
print("ok")
