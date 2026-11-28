#include <cstdio>
#include <unistd.h>
int main() {
  const char* m = "/tmp/bepfx-flaky.marker";
  if (access(m, F_OK) == 0) { unlink(m); printf("pass now\n"); return 0; }
  FILE* f = fopen(m, "w"); if (f) fclose(f);
  printf("FAIL first\n"); return 1;
}
