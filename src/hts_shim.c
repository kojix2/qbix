#include <stdint.h>
#include <stddef.h>
#include <stdlib.h>
#include <htslib/bgzf.h>
#include <htslib/hts.h>
#include <htslib/kstring.h>
#include <htslib/sam.h>
#include <htslib/tbx.h>

int64_t qbix_hts_shim_bgzf_tell(htsFile *fp)
{
    BGZF *bgzf = hts_get_bgzfp(fp);
    return bgzf == 0 ? -1 : bgzf_tell(bgzf);
}

int64_t qbix_hts_shim_bgzf_seek(htsFile *fp, int64_t offset)
{
    BGZF *bgzf = hts_get_bgzfp(fp);
    return bgzf == 0 ? -1 : bgzf_seek(bgzf, offset, SEEK_SET);
}

int qbix_hts_shim_bgzf_set_cache_size(htsFile *fp, int size)
{
    BGZF *bgzf = hts_get_bgzfp(fp);
    if (bgzf == 0) return -1;
    bgzf_set_cache_size(bgzf, size);
    return 0;
}

const char *qbix_hts_shim_bam_qname(bam1_t *b)
{
    return bam_get_qname(b);
}

const char *qbix_hts_shim_sam_hdr_text(const sam_hdr_t *h)
{
    return sam_hdr_str((sam_hdr_t *)h);
}

size_t qbix_hts_shim_sam_hdr_text_len(const sam_hdr_t *h)
{
    return sam_hdr_length((sam_hdr_t *)h);
}

int qbix_hts_shim_sam_format1(const sam_hdr_t *h, const bam1_t *b, char **out, size_t *len)
{
    kstring_t str = {0, 0, 0};
    if (out == 0 || len == 0) return -1;
    *out = 0;
    *len = 0;
    if (sam_format1(h, b, &str) < 0) {
        free(str.s);
        return -1;
    }
    *out = str.s;
    *len = str.l;
    return 0;
}

void qbix_hts_shim_free(void *ptr)
{
    free(ptr);
}
