#include <cstdio>
#include <cstdlib>
#include <sys/stat.h>
int main(){const char*m="/tmp/bepfx-fl2.marker";struct stat st;if(stat(m,&st)==0){std::remove(m);return 0;}FILE*f=fopen(m,"w");if(f)fclose(f);std::puts("flaky first");return 1;}
