#ifndef QBIX_H
#define QBIX_H

#include <stdint.h>
#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct qbix_index_t qbix_index_t;

typedef struct qbix_hit_t {
    int64_t virtual_offset;
} qbix_hit_t;

typedef enum qbix_check_mode_t {
    QBIX_CHECK_QUICK = 0,
    QBIX_CHECK_FULL = 1
} qbix_check_mode_t;

int qbix_build_index(const char *bam_path, const char *index_path, size_t threads);
int qbix_check_index(const char *bam_path,
                     const char *index_path,
                     size_t threads,
                     qbix_check_mode_t mode);

/* qbix_index_t handles are not thread-safe. */
qbix_index_t *qbix_index_open(const char *bam_path, const char *index_path, size_t threads);
int qbix_index_lookup(qbix_index_t *index,
                      const char *read_name,
                      qbix_hit_t **hits_out,
                      size_t *hit_count_out);
void qbix_hits_free(qbix_hit_t *hits, size_t hit_count);
void qbix_index_close(qbix_index_t *index);

/* Returns the last error for the current calling thread. */
const char *qbix_last_error(void);
const char *qbix_version(void);

#ifdef __cplusplus
}
#endif

#endif
