/* libfuzz: libFuzzer's coverage-guided fuzzing of our Vorbis decoder (../vorbisdec.c), fed as its callers feed it:
 * vorbis_init on the identification and setup headers, then one audio packet per call, a restart now and then.
 *
 * The input: [mode] [u16 LE length][identification] [u16 LE length][setup], then packets as [u16 LE length][bytes]
 * (the last one takes what is left if its length runs past the end; bit 15 of a length: vorbis_restart before the
 * packet, the length is the other 15 bits). mode bit 0: the setup's memory as zd1-spotify gives it (vorbis/rust: one
 * 192 KB block for both arenas, 1 MB if that is too small), else as Warp0's image does (fw/vorbis_main.c: a 10 KB
 * fast arena and a 128 KB slow one). The state, the arenas, every header and packet and the PCM are allocated to
 * exactly their size, so ASan sees any access outside them.
 *
 * Build:  clang -g -O1 -fsanitize=fuzzer,address,undefined -fno-sanitize-recover=undefined -I. -I../dsp \
 *            -o /tmp/vorbisfuzz check/libfuzz.c vorbisdec.c vorbis_tables.c ../dsp/imdct.c ../dsp/imdct_tables.c
 * Run:    /tmp/vorbisfuzz -max_len=32768 CORPUS_DIR   (a seed from `vorbischeck packets`: a mode byte, the
 *         identification and setup headers (the comment header left out), the audio packets, lengths cut to u16) */
#include <stdlib.h>
#include <string.h>
#include "vorbisdec.h"

struct in {
    const uint8_t *p;
    size_t size, at;
};

/* the next record, copied to a buffer of its exact length (freed by the caller); *flag = bit 15 of its length (the
 * length is the other 15 bits, the headers' too) */
static uint8_t *next(struct in *in, unsigned *len, int *flag) {
    size_t n = 0;
    *flag = 0;
    if (in->size - in->at >= 2) {
        n = in->p[in->at] | in->p[in->at + 1] << 8;
        *flag = n >> 15;
        n &= 0x7FFF;
        in->at += 2;
    } else
        in->at = in->size;
    if (n > in->size - in->at)
        n = in->size - in->at;
    uint8_t *b = malloc(n ? n : 1);
    memcpy(b, in->p + in->at, n);
    in->at += n;
    *len = (unsigned)n;
    return b;
}

int LLVMFuzzerTestOneInput(const uint8_t *data, size_t size) {
    if (size < 1)
        return 0;
    struct in in = {data, size, 1};
    int arm = data[0] & 1, flag;
    unsigned ilen, slen;
    uint8_t *ident = next(&in, &ilen, &flag), *setup = next(&in, &slen, &flag);
    struct vorbis_dec *d = malloc(vorbis_state_size());
    void *fast = NULL, *slow = NULL;
    int r = VORBIS_EMEM;
    if (arm) {
        for (unsigned kb = 192; r == VORBIS_EMEM && kb <= 1024; kb = kb == 192 ? 1024 : 2048) {
            free(fast);
            fast = malloc(kb << 10);
            r = vorbis_init(d, ident, ilen, setup, slen, fast, kb << 10, fast, kb << 10);
        }
    } else {
        fast = malloc(10240);
        slow = malloc(128 << 10);
        r = vorbis_init(d, ident, ilen, setup, slen, fast, 10240, slow, 128 << 10);
    }
    free(ident);
    free(setup);
    if (!r) {
        const struct vorbis_info *info = vorbis_stream_info(d);
        if (info->channels < 1 || info->channels > VORBIS_MAX_CH)
            abort();
        int16_t *pcm = malloc(VORBIS_MAX_OUT * info->channels * sizeof *pcm);
        while (in.at < in.size) {
            unsigned len;
            uint8_t *pkt = next(&in, &len, &flag);
            if (flag)
                vorbis_restart(d);
            int k = vorbis_decode(d, pkt, len, pcm);
            free(pkt);
            if (k > VORBIS_MAX_OUT)
                abort(); /* more samples than a packet can give */
        }
        free(pcm);
    }
    free(fast);
    free(slow);
    free(d);
    return 0;
}
