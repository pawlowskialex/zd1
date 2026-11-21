/* libfuzz: libFuzzer's coverage-guided fuzzing of our FLAC decoder (../flacdec.c), fed as zd1-snapcast's ARM decoder
 * feeds it (flac/rust: flac_init on the STREAMINFO body, then each chunk copied into a word buffer of exactly its
 * length rounded up to 4, decoded with flac_decode_from into room for max(max_block, 4608) frames until it is used up).
 *
 * The input: the 34-byte STREAMINFO body, then chunks as [u16 LE length][bytes] (the last one takes what is left if
 * its length runs past the end). The state, the input words and the output are allocated to exactly their size, so
 * ASan sees any access outside them (the decoder's contract: it may read the input up to len rounded up to 4).
 *
 * Build:  clang -g -O1 -fsanitize=fuzzer,address,undefined -fno-sanitize-recover=undefined -I. -I../dsp \
 *            -o /tmp/flacfuzz check/libfuzz.c flacdec.c   (Warp0's limits; the ARM's, zd1-snapcast's:
 *            -DFLAC_MAX_CH=8 -DFLAC_MAX_BLOCK=65535)
 * Run:    /tmp/flacfuzz -max_len=65536 CORPUS_DIR   (a seed: STREAMINFO, then a .flac file's first frames as one
 *         chunk, `flaccheck chunks` cut to u16 lengths) */
#include <stdlib.h>
#include <string.h>
#include "flacdec.h"

int LLVMFuzzerTestOneInput(const uint8_t *data, size_t size) {
    if (size < 34)
        return 0;
    struct flac_dec *d = malloc(flac_state_size());
    if (flac_init(d, data, 34)) {
        free(d);
        return 0;
    }
    const struct flac_info *info = flac_stream_info(d);
    uint32_t room = (info->max_block > 4608 ? info->max_block : 4608) * info->channels;
    int32_t *out = malloc(room * sizeof *out);
    for (size_t at = 34; at < size;) {
        size_t n = size - at < 2 ? 0 : (size_t)(data[at] | data[at + 1] << 8);
        at += size - at < 2 ? size - at : 2;
        if (n > size - at)
            n = size - at;
        uint32_t *in = calloc((n + 3) / 4 + (n == 0), 4);
        memcpy(in, data + at, n);
        for (uint32_t pos = 0; pos < n;) {
            uint32_t used, samples;
            int r = flac_decode_from(d, (const uint8_t *)in, (uint32_t)n, pos, out, room, &used, &samples);
            if (samples > room || used > n || (samples % info->channels))
                abort(); /* a result out of bounds */
            if (r || used <= pos)
                break;
            pos = used;
        }
        free(in);
        at += n;
    }
    free(out);
    free(d);
    return 0;
}
