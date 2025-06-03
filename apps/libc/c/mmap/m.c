#include <stdio.h>
#include <stdlib.h>
#include <sys/mman.h>
#include <sys/types.h>
#include <sys/stat.h>
#include <fcntl.h>
#include <unistd.h>
#include <string.h>

#define TEST_FILE_SIZE (1024 * 1024)  // 1MB 测试文件
#define PATTERN_BLOCK_SIZE 512      // 可预测数据块大小

// 生成可预测模式的文件内容
void generate_pattern(char* buf, size_t size) {
    for (size_t i = 0; i < size; i++) {
        buf[i] = (char)(i % 256);
    }
}

int main() {
    // 创建测试文件
    int fd = open("testfile.bin", O_RDWR | O_CREAT | O_TRUNC, 0644);
    if (fd < 0) {
        perror("open");
        exit(EXIT_FAILURE);
    }

    // 填充可验证内容
    char* file_content = malloc(TEST_FILE_SIZE);
    generate_pattern(file_content, TEST_FILE_SIZE);
    write(fd, file_content, TEST_FILE_SIZE);
    free(file_content);
    lseek(fd, 0, SEEK_SET);

    // 模拟你的系统调用序列
    // 注意：需要根据你的系统调用号调整参数

    // 1. 测试 read(3, buf, 832)
    struct stat st;
    if (fstat(fd, &st) == -1) {
        perror("fstat");
        exit(EXIT_FAILURE);
    }
    printf("File size: %ld bytes\n", st.st_size);

    char read_buf[832];
    ssize_t bytes_read = read(fd, read_buf, sizeof(read_buf));
    printf("Read %zd bytes\n", bytes_read);

    // 验证读取内容
    for (int i = 0; i < bytes_read; i++) {
        if (read_buf[i] != (char)(i % 256)) {
            fprintf(stderr, "Read data mismatch at byte %d\n", i);
            exit(EXIT_FAILURE);
        }
    }

    // 2. 测试第一个 mmap
    void* map1 = mmap(NULL, 0x4f4b8, PROT_READ | PROT_EXEC,
                     MAP_PRIVATE, fd, 0);
    printf("Map1 address: %p\n", map1);
    if (map1 == MAP_FAILED) {
        perror("mmap1");
        exit(EXIT_FAILURE);
    }

    // 3. 测试第二个 mmap (MAP_FIXED)
    void* map2 = mmap((void*)0x46000, 0x7000, PROT_READ | PROT_WRITE,
                     MAP_PRIVATE | MAP_FIXED, fd, 0x45000);
    printf("Map2 address: %p\n", map2);
    if (map2 == MAP_FAILED) {
        perror("mmap2");
        exit(EXIT_FAILURE);
    }

    // 4. 测试第三个 mmap (匿名映射)
    void* map3 = mmap((void*)0x4d000, 0x34b8, PROT_READ | PROT_WRITE,
                     MAP_PRIVATE | MAP_ANONYMOUS | MAP_FIXED, -1, 0);
    printf("Map3 address: %p\n", map3);
    if (map3 == MAP_FAILED) {
        perror("mmap3");
        exit(EXIT_FAILURE);
    }

    // 验证内存访问
    // 测试 map1 的读取和执行权限
    printf("Accessing map1...\n");
    char test1 = *(char*)map1;
    printf("Read from map1 succeeded\n");

    // 测试 map2 的写入能力
    printf("Writing to map2...\n");
    *(char*)map2 = 0xAA;
    printf("Write to map2 succeeded\n");

    // 测试 map3 的初始值
    printf("Checking map3 initialization...\n");
    for (int i = 0; i < 100; i++) {
        if (((char*)map3)[i] != 0) {
            fprintf(stderr, "Anonymous map not zero-initialized at %d\n", i);
            exit(EXIT_FAILURE);
        }
    }

    // 清理
    munmap(map1, 0x4f4b8);
    munmap(map2, 0x7000);
    munmap(map3, 0x34b8);
    close(fd);

    printf("All tests passed!\n");
    return 0;
}