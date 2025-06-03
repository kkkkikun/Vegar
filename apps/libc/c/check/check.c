#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#define EXPECTED_SIZE 4096
#define BLOCK_SIZE    1024
#define EXPECTED_BYTE 0x70

int main() {
    // 打开二进制文件
    FILE *fp = fopen("io.tmp", "rb");
    if (!fp) {
        perror("文件打开失败");
        return EXIT_FAILURE;
    }

    // 验证文件大小
    fseek(fp, 0, SEEK_END);
    long file_size = ftell(fp);
    rewind(fp);

    if (file_size != EXPECTED_SIZE) {
        fprintf(stderr, "文件大小错误：期望值 %d，实际值 %ld\n",
                EXPECTED_SIZE, file_size);
        fclose(fp);
        return EXIT_FAILURE;
    }

    // 创建验证缓冲区
    unsigned char buffer[BLOCK_SIZE];
    memset(buffer, ~EXPECTED_BYTE, BLOCK_SIZE); // 预填充非期望值

    // 逐块验证内容
    for (int i = 0; i < EXPECTED_SIZE / BLOCK_SIZE; i++) {
        // 读取数据块
        size_t read = fread(buffer, 1, BLOCK_SIZE, fp);
        if (read != BLOCK_SIZE) {
            perror("文件读取失败");
            fclose(fp);
            return EXIT_FAILURE;
        }

        // 验证块内容
        for (int j = 0; j < BLOCK_SIZE; j++) {
            if (buffer[j] != EXPECTED_BYTE) {
                fprintf(stderr, "数据验证失败：块 %d 偏移 %d 发现 0x%02X\n",
                        i, j, buffer[j]);
                fclose(fp);
                return EXIT_FAILURE;
            }
        }
    }

    fclose(fp);
    printf("文件验证成功：所有 %d 字节均为 0x%02X\n",
           EXPECTED_SIZE, EXPECTED_BYTE);
    return EXIT_SUCCESS;
}