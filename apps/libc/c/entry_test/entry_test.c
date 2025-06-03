#define _GNU_SOURCE
#include <stdio.h>
#include <stdint.h>
#include <elf.h>

// 辅助向量类型到字符串的映射
const char* get_aux_type_name(uint64_t type) {
    switch (type) {
    case AT_NULL: return "AT_NULL";
    case AT_PHDR: return "AT_PHDR (程序头表地址)";
    case AT_PHENT: return "AT_PHENT (程序头表条目大小)";
    case AT_PHNUM: return "AT_PHNUM (程序头表条目数量)";
    case AT_ENTRY: return "AT_ENTRY (程序入口点地址)";
    case AT_PAGESZ: return "AT_PAGESZ (系统页大小)";
    case AT_BASE: return "AT_BASE (解释器基地址)";
    case AT_FLAGS: return "AT_FLAGS (标志位)";
    case AT_UID: return "AT_UID (真实用户ID)";
    case AT_EUID: return "AT_EUID (有效用户ID)";
    case AT_GID: return "AT_GID (真实组ID)";
    case AT_EGID: return "AT_EGID (有效组ID)";
    case AT_PLATFORM: return "AT_PLATFORM (硬件平台名称)";
    case AT_RANDOM: return "AT_RANDOM (随机数地址)";
    case AT_EXECFN: return "AT_EXECFN (程序路径名)";
    case AT_SYSINFO_EHDR: return "AT_SYSINFO_EHDR (VDSO页地址)";
    default: return "UNKNOWN";
    }
}

int main(int argc, char *argv[], char *envp[]) {
    // 输出命令行参数
    printf("=== 命令行参数 (argc=%d) ===\n", argc);
    for (int i = 0; i < argc; i++) {
        printf("argv[%d] = %s, addr=%lx\n", i, argv[i], (unsigned long)argv[i]);
    }

    // 输出环境变量
    printf("\n=== 环境变量 ===\n");
    for (char **env = envp; *env != NULL; env++) {
        printf("%s\n", *env);
    }

    // 定位并输出辅助向量 (auxv)
    printf("\n=== 辅助向量 (auxv) ===\n");
    char **env_ptr;
    for (env_ptr = envp; *env_ptr != NULL; env_ptr++); // 跳过所有环境变量
    env_ptr++; // 跳过 NULL

    Elf64_auxv_t *auxv = (Elf64_auxv_t *)env_ptr; // auxv 起始地址
    while (auxv->a_type != AT_NULL) {
        const char *type_name = get_aux_type_name(auxv->a_type);
        printf("类型: %-30s 值: 0x%016lx\n", type_name, auxv->a_un.a_val);
        auxv++;
    }

    return 0;
}