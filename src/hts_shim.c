#include <stdint.h>
#include <stddef.h>
#include <htslib/bgzf.h>
#include <htslib/hts.h>
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
    return sam_hdr_str(h);
}

size_t qbix_hts_shim_sam_hdr_text_len(const sam_hdr_t *h)
{
    return sam_hdr_length(h);
}
