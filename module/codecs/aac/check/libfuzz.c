/* libfuzz: libFuzzer's coverage-guided fuzzing of our AAC-LC decoder (../aacdec.c), fed as zd1-airplay feeds it
 * (aac/rust: aac_init with the sender's rate and channels, then one raw access unit per call).
 *
 * The input: [rate index (mod 13)] [channels: 1 + bit 0], then access units as [u16 LE length][bytes] (the last one
 * takes what is left if its length runs past the end). Each unit gets a buffer of exactly its length, the state and
 * the PCM exactly theirs (the Rust binding's), so ASan sees any access outside them.
 *
 * Build:  clang -g -O1 -fsanitize=fuzzer,address,undefined -fno-sanitize-recover=undefined -I. -I../dsp \
 *            -o /tmp/aacfuzz check/libfuzz.c aacdec.c aac_tables.c ../dsp/imdct.c ../dsp/imdct_tables.c
 * Run:    /tmp/aacfuzz -max_len=8192 CORPUS_DIR   (a seed from FILE.frames: tools/m4a2frames.py's records, each
 *         u32 length cut to u16, after the two bytes) */
#include <stdlib.h>
#include <string.h>
#include "aacdec.h"

int LLVMFuzzerTestOneInput(const uint8_t *data, size_t size) {
    static const unsigned rates[13] = {96000, 88200, 64000, 48000, 44100, 32000, 24000,
                                       22050, 16000, 12000, 11025, 8000,  7350};
    if (size < 2)
        return 0;
    unsigned rate = rates[data[0] % 13], ch = 1 + (data[1] & 1);
    struct aac_dec *d = malloc(aac_state_size());
    int16_t *pcm = malloc(AAC_FRAME * ch * sizeof *pcm);
    if (aac_init(d, rate, ch))
        abort();
    for (size_t at = 2; at < size;) {
        size_t n = size - at < 2 ? 0 : (size_t)(data[at] | data[at + 1] << 8);
        at += size - at < 2 ? size - at : 2;
        if (n > size - at)
            n = size - at;
        uint8_t *au = malloc(n ? n : 1);
        memcpy(au, data + at, n);
        int r = aac_decode(d, au, (unsigned)n, pcm);
        free(au);
        if (r >= 0 && r != (int)(AAC_FRAME * ch))
            abort(); /* a frame is 1024 samples per channel or an error */
        at += n;
    }
    free(pcm);
    free(d);
    return 0;
}
