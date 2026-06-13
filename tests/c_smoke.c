#include <stdio.h>
#include <string.h>

#include "qbix.h"

int main(void)
{
    const char *version = qbix_version();
    if (version == 0 || version[0] == '\0') {
        fprintf(stderr, "qbix_version returned an empty value\n");
        return 1;
    }

    if (qbix_build_index(0, 0, 1) != -1) {
        fprintf(stderr, "qbix_build_index accepted a null BAM path\n");
        return 1;
    }

    const char *error = qbix_last_error();
    if (error == 0 || strstr(error, "bam_path is null") == 0) {
        fprintf(stderr, "unexpected qbix_last_error: %s\n", error == 0 ? "(null)" : error);
        return 1;
    }

    return 0;
}
