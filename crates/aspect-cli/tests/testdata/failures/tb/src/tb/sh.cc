#include <cstdlib>
#include <cstdio>
int main(){const char*s=getenv("TEST_SHARD_INDEX");const char*f=getenv("TEST_SHARD_STATUS_FILE");if(f){FILE*x=fopen(f,"w");if(x)fclose(x);}if(s&&s[0]==0x31){std::puts("shard 1 fails");return 1;}return 0;}
