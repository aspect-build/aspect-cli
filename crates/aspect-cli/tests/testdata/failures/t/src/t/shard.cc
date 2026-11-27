#include <cstdio>
#include <cstdlib>
#include <cstring>
int main() { const char* s = getenv("TEST_SHARD_INDEX"); const char* f = getenv("TEST_SHARD_STATUS_FILE"); if (f) { FILE* x = fopen(f, "w"); if (x) fclose(x); } printf("shard %s\n", s ? s : "?"); return (s && strcmp(s, "1") == 0) ? 1 : 0; }
