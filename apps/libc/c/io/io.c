#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#define BLOCK_SIZE 1024
#define TOTAL_BLOCKS 4

int main() {

    int a,b;
    printf("请输入2个整数：\n");
    scanf("%d%d", &a, &b);
    printf("a=%d, b=%d\n", a, b);
    // 打开文件时强制禁用缓冲
    FILE *fp = fopen("io.tmp", "wb");
    if (!fp) {
        perror("fopen() 失败");
        return EXIT_FAILURE;
    }

    // 关键设置：禁用标准库缓冲
    setvbuf(fp, NULL, _IONBF, 0);

    // 初始化缓冲区
    unsigned char buffer[BLOCK_SIZE];
    memset(buffer, 0x70, BLOCK_SIZE);

    // 分四次写入，确保每次都会触发系统调用
    for (int i = 0; i < TOTAL_BLOCKS; i++) {
        const size_t written = fwrite(buffer, 1, BLOCK_SIZE, fp);

        // 立即强制写入磁盘（如果需要更强的保证）
        if (fflush(fp) != 0) {
            perror("fflush() 失败");
            fclose(fp);
            return EXIT_FAILURE;
        }

        if (written != BLOCK_SIZE) {
            perror("fwrite() 失败");
            fclose(fp);
            return EXIT_FAILURE;
        }

        printf("已执行第 %d 次系统调用，写入 %zd 字节\n",
               i+1, written);
    }

    fclose(fp);
    printf("文件写入完成，共 %d 次系统调用\n", TOTAL_BLOCKS);
    return EXIT_SUCCESS;
}