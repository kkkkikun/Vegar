#include <time.h>
#include <errno.h>
#include <string.h>
#include <stdio.h>

int main()
{
    struct timespec ts;
    int ret = clock_gettime(CLOCK_REALTIME, &ts);
    int err = errno;
    printf("clock_gettime(CLOCK_REALTIME) = %d, errno = %d\n", ret, errno);
    return ret;
}