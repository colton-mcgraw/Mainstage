CC ?= gcc
CFLAGS ?= -fPIC -O2

# Default builds a Linux-style shared library. Pick a platform target explicitly:
#  make all-linux
#  make all-macos
#  make all-win

ENTRY := c_plugin_example

all: all-linux

all-linux:
	$(CC) $(CFLAGS) -shared -o $(ENTRY).so plugin.c

all-macos:
	$(CC) $(CFLAGS) -shared -o $(ENTRY).dylib plugin.c

all-win:
	$(CC) -shared -o $(ENTRY).dll plugin.c

clean:
	rm -f $(ENTRY).so $(ENTRY).dylib $(ENTRY).dll
