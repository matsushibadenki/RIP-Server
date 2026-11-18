/* Observe successful bitmap rasterization in the existing MuPDF draw process. */
#define _GNU_SOURCE
#include <dlfcn.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <openssl/sha.h>
#include <ft2build.h>
#include FT_FREETYPE_H

#define MAX_FACES 256
#define MAX_PAIRS 2048
#define PAIR_SLOTS (MAX_PAIRS * 2)

static struct {
    FT_Face face;
    unsigned char digest[SHA256_DIGEST_LENGTH];
    long index;
} fonts[MAX_FACES];

static struct {
    unsigned char digest[SHA256_DIGEST_LENGTH];
    long index;
    unsigned int gid;
} pairs[MAX_PAIRS];

static unsigned short index_by_pair[PAIR_SLOTS];
static size_t count;
static int truncated;

FT_Error FT_New_Memory_Face(FT_Library library, const FT_Byte *bytes,
                           FT_Long size, FT_Long index, FT_Face *face)
{
    static FT_Error (*real)(FT_Library, const FT_Byte *, FT_Long, FT_Long, FT_Face *);
    if (!real)
        real = dlsym(RTLD_NEXT, "FT_New_Memory_Face");
    FT_Error result = real(library, bytes, size, index, face);
    if (result || size <= 0 || size > 32 * 1024 * 1024)
        return result;
    for (size_t i = 0; i < MAX_FACES; i++) {
        if (!fonts[i].face) {
            fonts[i].face = *face;
            fonts[i].index = index;
            SHA256(bytes, (size_t)size, fonts[i].digest);
            break;
        }
    }
    return result;
}

FT_Error FT_Done_Face(FT_Face face)
{
    static FT_Error (*real)(FT_Face);
    if (!real)
        real = dlsym(RTLD_NEXT, "FT_Done_Face");
    for (size_t i = 0; i < MAX_FACES; i++)
        if (fonts[i].face == face) {
            fonts[i].face = NULL;
            break;
        }
    return real(face);
}

FT_Error FT_Render_Glyph(FT_GlyphSlot slot, FT_Render_Mode mode)
{
    static FT_Error (*real)(FT_GlyphSlot, FT_Render_Mode);
    if (!real)
        real = dlsym(RTLD_NEXT, "FT_Render_Glyph");
    FT_Error result = real(slot, mode);
    if (result)
        return result;
    for (size_t i = 0; i < MAX_FACES; i++) {
        if (fonts[i].face != slot->face)
            continue;
        uint32_t hash;
        memcpy(&hash, fonts[i].digest, sizeof(hash));
        hash ^= slot->glyph_index * UINT32_C(2654435761) + (uint32_t)fonts[i].index;
        size_t bucket = hash % PAIR_SLOTS;
        while (index_by_pair[bucket]) {
            size_t j = index_by_pair[bucket] - 1;
            if (pairs[j].index == fonts[i].index && pairs[j].gid == slot->glyph_index &&
                !memcmp(pairs[j].digest, fonts[i].digest, SHA256_DIGEST_LENGTH))
                return result;
            bucket = (bucket + 1) % PAIR_SLOTS;
        }
        if (count == MAX_PAIRS)
            break;
        index_by_pair[bucket] = (unsigned short)(count + 1);
        memcpy(pairs[count].digest, fonts[i].digest, SHA256_DIGEST_LENGTH);
        pairs[count].index = fonts[i].index;
        pairs[count].gid = slot->glyph_index;
        count++;
        return result;
    }
    truncated = 1;
    return result;
}

__attribute__((destructor)) static void write_report(void)
{
    const char *path = getenv("JRIP_RASTER_FONT_REPORT");
    if (!path)
        return;
    FILE *out = fopen(path, "wx");
    if (!out)
        return;
    fprintf(out, "{\"profile\":\"org.jrip.mupdf-bitmap-glyphs-v1\",\"truncated\":%s,\"glyphs\":[",
            truncated ? "true" : "false");
    for (size_t i = 0; i < count; i++) {
        fprintf(out, "%s[\"", i ? "," : "");
        for (size_t j = 0; j < SHA256_DIGEST_LENGTH; j++)
            fprintf(out, "%02x", pairs[i].digest[j]);
        fprintf(out, "\",%ld,%u]", pairs[i].index, pairs[i].gid);
    }
    fputs("]}\n", out);
    fclose(out);
}
