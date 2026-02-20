#include <stdio.h>
#include <stdlib.h>
#include <stdint.h>
#include <time.h>
#include <pcap.h>
#include <unistd.h>

// DPDK Includes
#include <rte_eal.h>
#include <rte_member.h>
#include <rte_errno.h>
#include <rte_cycles.h>
#include <rte_lcore.h>
#include <rte_log.h> 

#ifndef SOCKET_ID_ANY
#define SOCKET_ID_ANY -1
#endif

// 10亿个包 = 约 3.8GB 内存。确保你机器够大。
#define MAX_PKTS 1000000000
#define SKETCH_KEY_LEN 4 

// 计时辅助
double get_time_sec() {
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return ts.tv_sec + ts.tv_nsec * 1e-9;
}

// 封装测试逻辑
void run_test(const char *name, double sample_rate, uint32_t *keys, long count) {
    printf("--- Test: %s (Internal Sample Rate: %.4f) ---\n", name, sample_rate);

    struct rte_member_parameters params = {
        .name = "test_sketch",
        .type = RTE_MEMBER_TYPE_SKETCH,
        .key_len = SKETCH_KEY_LEN,
        // [关键调整] 针对大数据集，必须调大 Top-K 表容量，否则全是在处理碰撞
        .num_keys = 10000000, 
        .prim_hash_seed = 42,
        .error_rate = 0.001,   
        .top_k = 512,          
        .socket_id = SOCKET_ID_ANY,
        .sample_rate = sample_rate 
    };

    // 清理旧实例
    struct rte_member_setsum *old = rte_member_find_existing("test_sketch");
    if (old) rte_member_free(old);

    struct rte_member_setsum *setsum = rte_member_create(&params);
    if (setsum == NULL) {
        printf("[Error] Failed to create sketch. Err: %s\n", rte_strerror(rte_errno));
        return;
    }

    double start = get_time_sec();
    
    // 核心测试循环
    for (long i = 0; i < count; i++) {
        rte_member_add(setsum, &keys[i], 1);
    }

    double end = get_time_sec();
    double duration = end - start;
    
    printf("Processed: %ld packets\n", count);
    printf("Time:      %.6f s\n", duration);
    printf("Throughput: %.2f Mops/sec\n", (count / duration) / 1e6);
    
    rte_member_free(setsum);
    printf("\n");
}

// To compile: $ gcc -O3 bench_sketch_multi.c -o bench_sketch_multi $(pkg-config --cflags --libs libdpdk) -lpcap
// To run: $ sudo ./bench_sketch_multi ./data/*.pcap

int main(int argc, char *argv[]) {
    // 1. EAL Init
    char *fake_argv[] = {"bench_sketch", "--no-huge", NULL};
    int fake_argc = 2;
    if (rte_eal_init(fake_argc, fake_argv) < 0) 
        rte_exit(EXIT_FAILURE, "EAL Init Failed\n");

    // 屏蔽日志
    rte_log_set_global_level(RTE_LOG_EMERG);

    if (argc < 2) {
        fprintf(stderr, "Usage: %s <pcap_file1> [pcap_file2 ...]\n", argv[0]);
        return -1;
    }

    // 2. Load Data (Multiple Files)
    setbuf(stdout, NULL); 
    
    // 分配 huge memory (约 4GB)
    printf("Allocating memory for up to %d packets...\n", MAX_PKTS);
    uint32_t *keys = malloc(sizeof(uint32_t) * MAX_PKTS);
    if (!keys) {
        fprintf(stderr, "Malloc failed! Do you have 4GB+ free RAM?\n");
        return -1;
    }

    long total_count = 0;
    char errbuf[PCAP_ERRBUF_SIZE];

    // 循环读取所有参数传入的文件
    for (int i = 1; i < argc; i++) {
        if (total_count >= MAX_PKTS) break;
        
        printf("Reading %s ... ", argv[i]);
        pcap_t *handle = pcap_open_offline(argv[i], errbuf);
        if (!handle) { 
            fprintf(stderr, "\n[Warning] Could not open %s: %s\n", argv[i], errbuf); 
            continue; 
        }

        struct pcap_pkthdr header;
        const u_char *packet;
        long file_count = 0;

        while ((packet = pcap_next(handle, &header)) != NULL) {
            if (total_count >= MAX_PKTS) break;
            
            if (header.len >= 26 + 4) { 
                keys[total_count++] = *(uint32_t *)(packet + 26);
                file_count++;
            }
        }
        pcap_close(handle);
        printf("Added %ld packets.\n", file_count);
    }

    printf("\nTotal Loaded: %ld packets.\n\n", total_count);

    if (total_count == 0) {
        printf("No packets loaded. Exiting.\n");
        free(keys);
        return 0;
    }

    // 3. Run Tests
    run_test("Full Sampling (1.0)", 1.0, keys, total_count);
    run_test("Sampling (0.01)", 0.01, keys, total_count);

    free(keys);
    rte_eal_cleanup();
    return 0;
}