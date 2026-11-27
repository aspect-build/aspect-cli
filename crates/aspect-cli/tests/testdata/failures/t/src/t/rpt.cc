#include <cstdio>
#include <sys/stat.h>
int main() { if (mkdir("/tmp/bepfx-rpt.lock", 0700) == 0) { printf("rpt run failed\n"); return 1; } printf("rpt ok\n"); return 0; }
