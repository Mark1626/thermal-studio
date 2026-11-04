"""Open the Classic-BT SPP port, send read-only LuckPrinter queries, print any reply.
Usage: python3 -I spp_query.py /dev/cu.SeznikMiniX_XXXX
No print/feed commands are sent."""
import os, sys, time, termios, select

port = sys.argv[1]
t0 = time.time()
def ts(): return f"{time.time()-t0:6.3f}s"

print(f"{ts()} opening {port} (this pages the printer over RFCOMM)…", flush=True)
fd = os.open(port, os.O_RDWR | os.O_NOCTTY)
attrs = termios.tcgetattr(fd)
attrs[0] = 0; attrs[1] = 0; attrs[3] = 0          # raw iflag/oflag/lflag
attrs[2] |= termios.CLOCAL | termios.CREAD
termios.tcsetattr(fd, termios.TCSANOW, attrs)
print(f"{ts()} open ok", flush=True)

def drain(secs):
    end = time.time() + secs
    got = b""
    while time.time() < end:
        r, _, _ = select.select([fd], [], [], 0.1)
        if r:
            chunk = os.read(fd, 1024)
            if chunk: got += chunk
    return got

def show(b):
    if not b: return "(no reply)"
    return "[" + " ".join(f"{x:02X}" for x in b) + "]  \"" + "".join(chr(x) if 32 <= x < 127 else "." for x in b) + "\""

print(f"{ts()} initial bytes: {show(drain(1.5))}", flush=True)
queries = [
    ("model    10 FF 20 F0", b"\x10\xff\x20\xf0"),
    ("firmware 10 FF 20 F1", b"\x10\xff\x20\xf1"),
    ("battery  10 FF 50 F1", b"\x10\xff\x50\xf1"),
    ("status   10 FF 40",    b"\x10\xff\x40"),
    ("ESC/POS DLE EOT 1 (status)", b"\x10\x04\x01"),
    ("ESC/POS GS I 1 (model id)",  b"\x1d\x49\x01"),
]
for label, q in queries:
    os.write(fd, q)
    print(f"{ts()} -> {label}: {show(drain(1.5))}", flush=True)
os.close(fd)
print(f"{ts()} closed")
