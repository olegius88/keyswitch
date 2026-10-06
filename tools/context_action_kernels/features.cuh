// CUDA featurizer, part 4: context_action_features.extract_action_features. A feature name is
// built as its UTF-8 bytes and kept as two independent 64-bit hashes; the strings themselves are
// written only in name mode, for the names the host asks for.
#pragma once
#include "evidence.cuh"

struct FeatureTables {
    HashTable term[KS_CONTEXT_ACTION_CUDA_TERM_TABLES_COUNT];   // context-term-frequency tables, TERM_* order
    u64 term_counts[KS_CONTEXT_ACTION_CUDA_TERM_TABLES_COUNT];  // i64 count per key
    i64 bucket_bounds[KS_CONTEXT_ACTION_CUDA_SMALL_TABLE_CAPACITY]; i64 bucket_count;
    HashTable logfrequency;  // decimal frequency -> index; value log1p(min(cap, f)) / log1p(cap)
    u64 logfrequency_value;
    i64 ngram_orders[KS_CONTEXT_ACTION_CUDA_SMALL_TABLE_CAPACITY]; i64 ngram_count;
};

struct Record {              // the ContextEvidence fields the features read
    double source[KS_CONTEXT_ACTION_CUDA_SCORE_FIELDS_COUNT], target[KS_CONTEXT_ACTION_CUDA_SCORE_FIELDS_COUNT];  // SCORE_* order
    i64 source_frequency, target_frequency;
    i64 source_flags, target_flags;     // SF_* bits
    double score_delta, probability, threshold, ortho_score, ortho_threshold;
    i64 flags;               // RF_* bits (CONTEXT_ACTION_CUDA_RECORD_FLAGS)
    i64 source_group, after_origin;     // ORIGIN_* (context_model.AfterOrigin order)
};

struct Text { const cp* s; int n; };
struct FeatureRow {          // strings of one evidence
    Text original, alternative, before, after, application, role, trigger, tail, boundary;
};

#define NAME_BYTES KS_CONTEXT_ACTION_CUDA_NAME_BYTES
struct Name {
    unsigned char b[NAME_BYTES]; int n; bool overflow;
    __device__ void clear() { n = 0; overflow = false; }
    __device__ void byte(unsigned char c) { if (n < NAME_BYTES) b[n++] = c; else overflow = true; }
    __device__ void ascii(const char* s) { while (*s) byte((unsigned char)*s++); }
    __device__ void ch(cp c) {
        if (c < 0x80u) byte((unsigned char)c);
        else if (c < 0x800u) { byte(0xC0u | (c >> 6)); byte(0x80u | (c & 0x3Fu)); }
        else if (c < 0x10000u) { byte(0xE0u | (c >> 12)); byte(0x80u | ((c >> 6) & 0x3Fu)); byte(0x80u | (c & 0x3Fu)); }
        else { byte(0xF0u | (c >> 18)); byte(0x80u | ((c >> 12) & 0x3Fu)); byte(0x80u | ((c >> 6) & 0x3Fu)); byte(0x80u | (c & 0x3Fu)); }
    }
    __device__ void text(const cp* s, int len) { for (int i = 0; i < len; ++i) ch(s[i]); }
    __device__ void integer(long long v) {
        char digits[KS_CONTEXT_ACTION_CUDA_DECIMAL_DIGITS]; int k = 0; bool negative = v < 0; unsigned long long u = negative ? (unsigned long long)(-v) : (unsigned long long)v;
        do { digits[k++] = (char)('0' + u % 10); u /= 10; } while (u);
        if (negative) byte('-');
        while (k) byte((unsigned char)digits[--k]);
    }
    __device__ void hex4(unsigned v) {     // f"{v:04x}"
        char digits[8]; int k = 0;
        do { unsigned d = v & 15u; digits[k++] = (char)(d < 10 ? '0' + d : 'a' + d - 10); v >>= 4; } while (v);
        while (k < 4) digits[k++] = '0';
        while (k) byte((unsigned char)digits[--k]);
    }
    __device__ void hash(u64* h1, u64* h2) const {
        // FNV-1a, and a second hash that tells two names with one FNV-1a apart (context_action_cuda.name_hashes)
        u64 a = (u64)KS_FNV1A64_OFFSET_BASIS, c = (u64)KS_CONTEXT_ACTION_NAME_HASH_SEED;
        for (int i = 0; i < n; ++i) {
            a ^= b[i]; a *= (u64)KS_FNV1A64_PRIME;
            c = c * (u64)KS_CONTEXT_ACTION_NAME_HASH_MULTIPLIER + b[i] + 1; c ^= c >> KS_CONTEXT_ACTION_NAME_HASH_SHIFT;
        }
        *h1 = a; *h2 = c ^ ((u64)n << KS_CONTEXT_ACTION_NAME_HASH_LENGTH_SHIFT);
    }
};

struct Wanted {              // name mode: the hashes to write, and where
    const u64* hashes; const i64* slots; int count;   // sorted by hash
    unsigned char* names; int* lengths;               // slot x NAME_BYTES
};

#define MAX_FEATURES KS_CONTEXT_ACTION_CUDA_FEATURES_PER_FRAME
#define FEATURE_SLOTS (1 << KS_CONTEXT_ACTION_CUDA_FEATURE_SLOT_BITS)
static_assert(MAX_FEATURES < FEATURE_SLOTS, "feature slots too few");
struct FeatureMap {
    u64 h1[MAX_FEATURES]; u64 h2[MAX_FEATURES]; double v[MAX_FEATURES]; short slots[FEATURE_SLOTS]; int n; bool overflow;
    const Wanted* wanted;
    __device__ void clear(const Wanted* w) { n = 0; overflow = false; wanted = w; for (int i = 0; i < FEATURE_SLOTS; ++i) slots[i] = -1; }
    __device__ int find(u64 a, u64 c, bool insert) {
        unsigned slot = (unsigned)(a >> (64 - KS_CONTEXT_ACTION_CUDA_FEATURE_SLOT_BITS));
        while (slots[slot] >= 0) {
            int i = slots[slot];
            if (h1[i] == a && h2[i] == c) return i;
            slot = (slot + 1) & (FEATURE_SLOTS - 1);
        }
        if (!insert) return -1;
        if (n >= MAX_FEATURES) { overflow = true; return -1; }
        slots[slot] = (short)n; h1[n] = a; h2[n] = c; v[n] = 0.0;
        return n++;
    }
    __device__ void remember(const Name& name, u64 a) {
        if (wanted == nullptr) return;
        int lo = 0, hi = wanted->count;
        while (lo < hi) { int mid = (lo + hi) / 2; if (wanted->hashes[mid] < a) lo = mid + 1; else hi = mid; }
        if (lo < wanted->count && wanted->hashes[lo] == a) {
            i64 slot = wanted->slots[lo];
            for (int i = 0; i < name.n; ++i) wanted->names[slot * NAME_BYTES + i] = name.b[i];
            wanted->lengths[slot] = name.n;
        }
    }
    __device__ void set(const Name& name, double value) {          // features[name] = value
        if (name.overflow) { overflow = true; return; }
        u64 a, c; name.hash(&a, &c);
        int i = find(a, c, true);
        if (i >= 0) { v[i] = value; remember(name, a); }
    }
    __device__ double get(const Name& name) {                      // features.get(name, 0.0)
        u64 a, c; name.hash(&a, &c);
        int i = find(a, c, false);
        return i >= 0 ? v[i] : 0.0;
    }
};

__device__ __forceinline__ bool bounded(double value, double lower, double upper, double* out) {   // _bounded
    if (!isfinite(value)) return false;
    *out = py_max(lower, py_min(upper, value));
    return true;
}
__device__ __forceinline__ bool ru_letter(cp c) { return (c >= 0x430u && c <= 0x44Fu) || c == 0x451u; }
__device__ __forceinline__ bool en_letter(cp c) { return c >= 'a' && c <= 'z'; }
__device__ void ascii_name(Name& name, const char* a) { name.clear(); name.ascii(a); }

// _script on already casefolded text
__device__ const char* script_of(const cp* s, int n) {
    bool ru = false, en = false;
    for (int i = 0; i < n; ++i) { if (ru_letter(s[i])) ru = true; if (en_letter(s[i])) en = true; }
    return ru && en ? "mixed" : ru ? "ru" : en ? "en" : "none";
}
// _dominant on casefolded text
__device__ const char* dominant(const cp* s, int n) {
    int ru = 0, en = 0;
    for (int i = 0; i < n; ++i) { ru += ru_letter(s[i]); en += en_letter(s[i]); }
    return ru > en ? "ru" : en > ru ? "en" : "none";
}

// context_model.term_bucket: -1 for "na", else bisect_right(bounds, table.get(core, 0)).
__device__ bool term_edge(cp c) {
    const char* signs = ".,!?:;\"'()[]{}<>-";
    if (c == 0xABu || c == 0xBBu) return true;
    for (const char* p = signs; *p; ++p) if (c == (cp)(unsigned char)*p) return true;
    return false;
}
__device__ int term_bucket(const Model* M, const FeatureTables* Ft, const cp* text, int n, int alphabet) {
    int start = 0, stop = n;
    while (start < stop && term_edge(text[start])) ++start;
    while (stop > start && term_edge(text[stop - 1])) --stop;
    int k = stop - start;
    if (k <= 0) return -1;
    cp core[MAXS];
    for (int i = 0; i < k; ++i) core[i] = casefold(M, text[start + i]);
    if (!str_isalpha(M, core, k)) return -1;
    int index = table_find(Ft->term[alphabet], core, k);
    i64 count = index >= 0 ? ((const i64*)Ft->term_counts[alphabet])[index] : 0;
    int bucket = 0;
    while (bucket < Ft->bucket_count && Ft->bucket_bounds[bucket] <= count) ++bucket;
    return bucket;
}
__device__ void bucket_name(Name& name, int bucket) { if (bucket < 0) name.ascii("na"); else name.integer(bucket); }

// _characters(features, label, text, direction)
__device__ void characters(FeatureMap& f, const FeatureTables* Ft, const char* label, const cp* text, int n, const char* direction) {
    if (n == 0) return;
    cp padded[MAXS + 2];
    padded[0] = '^';
    for (int i = 0; i < n; ++i) padded[i + 1] = text[i];
    padded[n + 1] = '$';
    int length = n + 2;
    Name name;
    for (int o = 0; o < Ft->ngram_count; ++o) {
        int order = (int)Ft->ngram_orders[o];
        int count = length - order + 1;
        for (int index = 0; index < count; ++index) {
            name.clear(); name.ascii(label); name.ascii(":char:"); name.ascii(direction); name.byte(':');
            name.integer(order); name.byte(':'); name.text(padded + index, order);
            double value = py_min(KS_ACTION_FEATURE_CHARACTER_WEIGHT_CAP, f.get(name) + 1.0 / sqrt((double)count));
            f.set(name, value);
        }
    }
}

__device__ bool logfrequency(const Model* M, const FeatureTables* Ft, i64 frequency, double* out) {
    i64 clamped = frequency < 0 ? 0 : frequency;
    if (clamped > (i64)KS_ACTION_FEATURE_FREQUENCY_CAP) clamped = (i64)KS_ACTION_FEATURE_FREQUENCY_CAP;
    if (clamped == 0) { *out = 0.0; return true; }   // log1p(0) / log1p(cap)
    cp digits[KS_CONTEXT_ACTION_CUDA_DECIMAL_DIGITS]; int k = 0; i64 v = clamped; cp reversed[KS_CONTEXT_ACTION_CUDA_DECIMAL_DIGITS];
    while (v) { reversed[k++] = (cp)('0' + v % 10); v /= 10; }
    for (int i = 0; i < k; ++i) digits[i] = reversed[k - 1 - i];
    int index = table_find(Ft->logfrequency, digits, k);
    if (index < 0) return false;
    *out = ((const double*)Ft->logfrequency_value)[index];
    return true;
}

// _word_score
__device__ bool word_score(FeatureMap& f, const FeatureTables* Ft, const Model* M, const char* label, const double* s, i64 frequency,
                           i64 flags, bool known) {
    Name name;
    name.clear(); name.ascii(label); name.ascii(":known:"); name.integer(known); f.set(name, 1.0);
    if (!(flags & SF_PRESENT)) { name.clear(); name.ascii(label); name.ascii(":score_missing"); f.set(name, 1.0); return true; }
    if (frequency < 0) return false;
    const double B = KS_ACTION_FEATURE_WORD_SCORE_BOUND, RB = KS_ACTION_FEATURE_RAW_NGRAM_SCORE_BOUND;
    double x;
    if (!bounded(s[SCORE_VALUE], -B, B, &x)) return false;
    name.clear(); name.ascii(label); name.ascii(":value"); f.set(name, x / B);
    if (!bounded(s[SCORE_GRAM_RATIO], 0.0, 1.0, &x)) return false;
    name.clear(); name.ascii(label); name.ascii(":gram_ratio"); f.set(name, x);
    if (!bounded(s[SCORE_NGRAM_SCORE], -B, B, &x)) return false;
    name.clear(); name.ascii(label); name.ascii(":ngram_score"); f.set(name, x / B);
    if (!bounded(s[SCORE_INVALID_RATIO], 0.0, 1.0, &x)) return false;
    name.clear(); name.ascii(label); name.ascii(":invalid_ratio"); f.set(name, x);
    if (!bounded(s[SCORE_RAW_NGRAM_SCORE], -RB, 0.0, &x)) return false;
    name.clear(); name.ascii(label); name.ascii(":raw_ngram_score"); f.set(name, x / RB);
    if (!logfrequency(M, Ft, frequency, &x)) return false;
    name.clear(); name.ascii(label); name.ascii(":logfrequency"); f.set(name, x);
    name.clear(); name.ascii(label); name.ascii(":exact:"); name.integer((flags & SF_EXACT) != 0); f.set(name, 1.0);
    name.clear(); name.ascii(label); name.ascii(":spell_known:"); name.integer((flags & SF_SPELL_KNOWN) != 0); f.set(name, 1.0);
    return true;
}

// Python re [^\W\d_]+(?:['’\-][^\W\d_]+)* on casefolded text
__device__ __forceinline__ bool word_letter(const Model* M, cp c) { return props(M, c) & P_WORD; }
__device__ int word_end(const Model* M, const cp* s, int n, int i) {
    int j = i;
    while (j < n && word_letter(M, s[j])) ++j;
    while (j + 1 < n && (s[j] == '\'' || s[j] == 0x2019u || s[j] == '-') && word_letter(M, s[j + 1])) {
        j += 1;
        while (j < n && word_letter(M, s[j])) ++j;
    }
    return j;
}

struct Bucket { int kind; int value; int extra; };   // kind 0 number(value; -1 na), 1 "lexicon", 2 "0:{extra}", 3 "lexicon:{extra}"
__device__ void bucket_text(Name& name, const Bucket& b) {
    if (b.kind == 0) bucket_name(name, b.value);
    else if (b.kind == 1) name.ascii("lexicon");
    else if (b.kind == 2) { name.ascii("0:"); bucket_name(name, b.extra); }
    else { name.ascii("lexicon:"); bucket_name(name, b.extra); }
}

// _term_features with _capitals_features and _frequency_features
__device__ void term_features(FeatureMap& f, const Model* M, const FeatureTables* Ft, const FeatureRow& row, const Record& r,
                              const char* direction) {
    const bool sk = r.flags & RF_SOURCE_KNOWN, tk = r.flags & RF_TARGET_KNOWN;
    cp before[MAXCTX], after[MAXCTX];
    int bstart = row.before.n > KS_ACTION_FEATURE_BEFORE_CONTEXT_CHARACTERS ? row.before.n - (int)KS_ACTION_FEATURE_BEFORE_CONTEXT_CHARACTERS : 0;
    int bn = row.before.n - bstart;
    for (int i = 0; i < bn; ++i) before[i] = casefold(M, row.before.s[bstart + i]);
    int an = row.after.n < KS_ACTION_FEATURE_AFTER_CONTEXT_CHARACTERS ? row.after.n : (int)KS_ACTION_FEATURE_AFTER_CONTEXT_CHARACTERS;
    for (int i = 0; i < an; ++i) after[i] = casefold(M, row.after.s[i]);
    // lines = before.rsplit("\n", 1) over the whole field.before
    int last_newline = -1;
    for (int i = row.before.n - 1; i >= 0; --i) if (row.before.s[i] == '\n') { last_newline = i; break; }
    bool last_alpha = false;
    for (int i = last_newline + 1; i < row.before.n; ++i) if (is_alpha(M, row.before.s[i])) { last_alpha = true; break; }
    const char* state = last_alpha ? "same" : last_newline >= 0 ? "newline" : "empty";
    Name name;
    name.clear(); name.ascii("line:"); name.ascii(state); name.ascii(":direction:"); name.ascii(direction); f.set(name, 1.0);
    name.clear(); name.ascii("line:"); name.ascii(state); name.ascii(":known:"); name.integer(sk); name.byte(':'); name.integer(tk);
    name.ascii(":direction:"); name.ascii(direction); f.set(name, 1.0);
    if (last_newline >= 0 && !last_alpha) {
        int previous_newline = -1;
        for (int i = last_newline - 1; i >= 0; --i) if (row.before.s[i] == '\n') { previous_newline = i; break; }
        cp previous[MAXCTX]; int pn = 0;
        for (int i = previous_newline + 1; i < last_newline; ++i) previous[pn++] = casefold(M, row.before.s[i]);
        name.clear(); name.ascii("line:newline:previous:"); name.ascii(dominant(previous, pn)); name.ascii(":direction:"); name.ascii(direction);
        f.set(name, 1.0);
    }
    const bool so = r.flags & RF_SOURCE_OPENING, to = r.flags & RF_TARGET_OPENING;
    if (so || to) {
        bool alone = !str_has_nonspace(M, before, bn) && !str_has_nonspace(M, after, an);
        name.clear(); name.ascii("opening:"); name.integer(so); name.byte(':'); name.integer(to); name.ascii(":direction:"); name.ascii(direction); f.set(name, 1.0);
        name.clear(); name.ascii("opening:"); name.integer(so); name.byte(':'); name.integer(to); name.ascii(":alone:"); name.integer(alone);
        name.ascii(":direction:"); name.ascii(direction); f.set(name, 1.0);
        name.clear(); name.ascii("opening:"); name.integer(so); name.byte(':'); name.integer(to); name.ascii(":line:"); name.ascii(state);
        name.ascii(":direction:"); name.ascii(direction); f.set(name, 1.0);
    }
    const Text latin = r.source_group == 0 ? row.original : row.alternative;
    const Text cyrillic = r.source_group == 0 ? row.alternative : row.original;
    bool dot = false;
    for (int i = 0; i + 2 < latin.n; ++i) {
        cp a = latin.s[i], c = latin.s[i + 2];
        bool la = (a >= 'A' && a <= 'Z') || (a >= 'a' && a <= 'z'), lc = (c >= 'A' && c <= 'Z') || (c >= 'a' && c <= 'z');
        if (la && latin.s[i + 1] == '.' && lc) { dot = true; break; }
    }
    name.clear(); name.ascii("latin:dot:direction:"); name.ascii(direction); f.set(name, dot ? 1.0 : 0.0);
    int us = 0, ue = latin.n;
    while (us < ue && latin.s[us] == '_') ++us;
    while (ue > us && latin.s[ue - 1] == '_') --ue;
    bool underscore = false;
    for (int i = us; i < ue; ++i) if (latin.s[i] == '_') { underscore = true; break; }
    name.clear(); name.ascii("latin:underscore:direction:"); name.ascii(direction); f.set(name, underscore ? 1.0 : 0.0);
    const bool cyrillic_known = r.source_group == 1 ? sk : tk;
    Bucket latin_bucket = {0, term_bucket(M, Ft, latin.s, latin.n, TERM_LATIN), 0};
    Bucket cyrillic_bucket = cyrillic_known ? Bucket{1, 0, 0} : Bucket{0, term_bucket(M, Ft, cyrillic.s, cyrillic.n, TERM_CYRILLIC), 0};
    // _capitals_features
    const Text& letters = row.original;
    if (letters.n >= KS_ACTION_FEATURE_CAPITALS_MIN_LETTERS && str_isalpha(M, letters.s, letters.n) && str_isupper(M, letters.s, letters.n)) {
        int russian = term_bucket(M, Ft, cyrillic.s, cyrillic.n, TERM_RUSSIAN);
        name.clear(); name.ascii("capitals:known:"); name.integer(cyrillic_known); name.ascii(":russian:"); bucket_name(name, russian);
        name.ascii(":direction:"); name.ascii(direction); f.set(name, 1.0);
        name.clear(); name.ascii("capitals:known:"); name.integer(cyrillic_known); name.ascii(":russian:"); bucket_name(name, russian);
        name.ascii(":before:"); name.ascii(dominant(before, bn)); name.ascii(":direction:"); name.ascii(direction); f.set(name, 1.0);
    }
    // _frequency_features
    bool latin_na = latin_bucket.kind == 0 && latin_bucket.value < 0;
    bool cyrillic_na = cyrillic_bucket.kind == 0 && cyrillic_bucket.value < 0;
    if (!latin_na || !cyrillic_na) {
        const bool planned = r.after_origin == ORIGIN_PLANNED, inside = r.flags & RF_INSIDE;
        Name context; context.clear();
        context.ascii("before:"); context.ascii(dominant(before, bn)); context.byte(':'); context.ascii(planned ? "next" : "after");
        context.byte(':'); context.ascii(dominant(after, an));
        if (!inside && (str_has_nonspace(M, before, bn) || str_has_nonspace(M, after, an))) {
            int english = term_bucket(M, Ft, latin.s, latin.n, TERM_ENGLISH);
            int russian = term_bucket(M, Ft, cyrillic.s, cyrillic.n, TERM_RUSSIAN);
            if (latin_bucket.kind == 0 && latin_bucket.value == 0) latin_bucket = Bucket{2, 0, english};
            if (cyrillic_known) cyrillic_bucket = Bucket{3, 0, russian};
            name.clear(); name.ascii("freq:english:"); bucket_name(name, english); name.ascii(":direction:"); name.ascii(direction); f.set(name, 1.0);
            name.clear(); name.ascii("freq:russian:"); bucket_name(name, russian); name.ascii(":direction:"); name.ascii(direction); f.set(name, 1.0);
            for (int variant = 0; variant < 3; ++variant) {
                name.clear(); name.ascii("freq:english:"); bucket_name(name, english); name.ascii(":russian:"); bucket_name(name, russian);
                if (variant == 1) { name.byte(':'); name.text(nullptr, 0); for (int i = 0; i < context.n; ++i) name.byte(context.b[i]); }
                if (variant == 2) { name.ascii(":line:"); name.ascii(state); }
                name.ascii(":direction:"); name.ascii(direction); f.set(name, 1.0);
            }
        }
        const char* frequency = inside ? "inside:freq" : "freq";
        name.clear(); name.ascii(frequency); name.ascii(":latin:"); bucket_text(name, latin_bucket); name.ascii(":direction:"); name.ascii(direction); f.set(name, 1.0);
        name.clear(); name.ascii(frequency); name.ascii(":cyrillic:"); bucket_text(name, cyrillic_bucket); name.ascii(":direction:"); name.ascii(direction); f.set(name, 1.0);
        name.clear(); name.ascii(frequency); name.byte(':'); bucket_text(name, latin_bucket); name.byte(':'); bucket_text(name, cyrillic_bucket);
        name.ascii(":direction:"); name.ascii(direction); f.set(name, 1.0);
        name.clear(); name.ascii(frequency); name.byte(':'); bucket_text(name, latin_bucket); name.byte(':'); bucket_text(name, cyrillic_bucket);
        name.byte(':'); for (int i = 0; i < context.n; ++i) name.byte(context.b[i]); name.ascii(":direction:"); name.ascii(direction); f.set(name, 1.0);
    }
    bool original_alpha = false, original_digit = false, alternative_alpha = false;
    for (int i = 0; i < row.original.n; ++i) { if (is_alpha(M, row.original.s[i])) original_alpha = true; if (is_digit(M, row.original.s[i])) original_digit = true; }
    for (int i = 0; i < row.alternative.n; ++i) if (is_alpha(M, row.alternative.s[i])) alternative_alpha = true;
    if (!original_alpha) {
        const char* kind = alternative_alpha ? "letters" : "only";
        name.clear(); name.ascii("signs:"); name.ascii(kind); name.ascii(":direction:"); name.ascii(direction); f.set(name, 1.0);
        name.clear(); name.ascii("signs:"); name.ascii(kind); name.ascii(":digits:"); name.integer(original_digit); name.ascii(":direction:"); name.ascii(direction);
        f.set(name, 1.0);
    }
}

// extract_action_features; false means Python would raise (the row then goes to the CPU path).
__device__ bool extract_action_features(FeatureMap& f, const Model* M, const FeatureTables* Ft, const FeatureRow& row, const Record& r) {
    if (r.source_group != 0 && r.source_group != 1) return false;
    const char* origins[ORIGIN_COUNT] = {"none", "field", "planned_next_conversion"};
    if (r.after_origin < 0 || r.after_origin >= ORIGIN_COUNT) return false;
    const char* origin = origins[r.after_origin];
    if (r.after_origin == ORIGIN_PLANNED) {
        bool space_trigger = row.trigger.n == 5 && row.trigger.s[0] == 's' && row.trigger.s[1] == 'p' && row.trigger.s[2] == 'a'
                             && row.trigger.s[3] == 'c' && row.trigger.s[4] == 'e';
        bool spaced = false;
        for (int i = 0; i < row.after.n; ++i) if (is_space(M, row.after.s[i])) spaced = true;
        if (!(row.original.n > 0 && row.original.n <= KS_PLANNED_CONTEXT_WORD_MAX_CHARACTERS) || !space_trigger
            || !(row.boundary.n == 1 && row.boundary.s[0] == ' ') || row.after.n == 0
            || row.after.n > KS_PLANNED_CONTEXT_AFTER_MAX_CHARACTERS || spaced) return false;
    }
    const char* direction = r.source_group == 0 ? "0" : "1";
    int length = row.original.n > row.alternative.n ? row.original.n : row.alternative.n;
    if (length > KS_ACTION_FEATURE_LENGTH_BUCKET_MAX_CHARACTERS) length = (int)KS_ACTION_FEATURE_LENGTH_BUCKET_MAX_CHARACTERS;
    const bool bc = r.flags & RF_BASELINE, sk = r.flags & RF_SOURCE_KNOWN, tk = r.flags & RF_TARGET_KNOWN;
    const bool si = r.flags & RF_SOURCE_IDENTIFIER, ti = r.flags & RF_TARGET_IDENTIFIER;
    const double B = KS_ACTION_FEATURE_WORD_SCORE_BOUND;
    Name name;
    double delta;
    if (!bounded(r.score_delta, -B, B, &delta)) return false;
    cp folded[MAXCTX];
    int an = row.after.n < KS_ACTION_FEATURE_AFTER_CONTEXT_CHARACTERS ? row.after.n : (int)KS_ACTION_FEATURE_AFTER_CONTEXT_CHARACTERS;
    for (int i = 0; i < an; ++i) folded[i] = casefold(M, row.after.s[i]);
    const char* after_script = script_of(folded, an);
    ascii_name(name, "bias"); f.set(name, 1.0);
    name.clear(); name.ascii("direction:"); name.ascii(direction); f.set(name, 1.0);
    name.clear(); name.ascii("baseline:"); name.integer(bc); f.set(name, 1.0);
    name.clear(); name.ascii("length:"); name.integer(length); f.set(name, 1.0);
    name.clear(); name.ascii("baseline:"); name.integer(bc); name.ascii(":length:"); name.integer(length); f.set(name, 1.0);
    name.clear(); name.ascii("known:"); name.integer(sk); name.byte(':'); name.integer(tk); name.ascii(":length:"); name.integer(length); f.set(name, 1.0);
    name.clear(); name.ascii("identifier:"); name.integer(si); name.byte(':'); name.integer(ti); f.set(name, 1.0);
    name.clear(); name.ascii("role:"); name.text(row.role.s, row.role.n < KS_ACTION_FEATURE_FIELD_LABEL_MAX_CHARACTERS ? row.role.n : (int)KS_ACTION_FEATURE_FIELD_LABEL_MAX_CHARACTERS); f.set(name, 1.0);
    name.clear(); name.ascii("trigger:"); name.text(row.trigger.s, row.trigger.n < KS_ACTION_FEATURE_FIELD_LABEL_MAX_CHARACTERS ? row.trigger.n : (int)KS_ACTION_FEATURE_FIELD_LABEL_MAX_CHARACTERS); f.set(name, 1.0);
    ascii_name(name, "score_delta"); f.set(name, delta / B);
    name.clear(); name.ascii("after_origin:"); name.ascii(origin); f.set(name, 1.0);
    name.clear(); name.ascii("after_origin:"); name.ascii(origin); name.ascii(":direction:"); name.ascii(direction); name.ascii(":length:");
    name.integer(length); f.set(name, 1.0);
    name.clear(); name.ascii("after_origin:"); name.ascii(origin); name.ascii(":script:"); name.ascii(after_script); name.ascii(":direction:");
    name.ascii(direction); name.ascii(":length:"); name.integer(length); f.set(name, 1.0);
    int bstart = row.before.n > KS_ACTION_FEATURE_BEFORE_CONTEXT_CHARACTERS ? row.before.n - (int)KS_ACTION_FEATURE_BEFORE_CONTEXT_CHARACTERS : 0;
    int bn = row.before.n - bstart;
    cp before_folded[MAXCTX];
    for (int i = 0; i < bn; ++i) before_folded[i] = casefold(M, row.before.s[bstart + i]);
    if (si || ti) {
        name.clear(); name.ascii("identifier:"); name.integer(si); name.byte(':'); name.integer(ti); name.ascii(":direction:"); name.ascii(direction); f.set(name, 1.0);
        name.clear(); name.ascii("identifier:"); name.integer(si); name.byte(':'); name.integer(ti); name.ascii(":length:"); name.integer(length); f.set(name, 1.0);
        name.clear(); name.ascii("identifier:"); name.integer(si); name.byte(':'); name.integer(ti); name.ascii(":before:"); name.ascii(script_of(before_folded, bn));
        f.set(name, 1.0);
    }
    const char* source_case = "none";
    const char* marks = ",.;[]'`-_/@\\=<>\"";
    for (int side = 0; side < 2; ++side) {
        const char* label = side == 0 ? "source" : "target";
        const Text& whole = side == 0 ? row.original : row.alternative;
        int rn = whole.n < KS_ACTION_FEATURE_WORD_MAX_CHARACTERS ? whole.n : (int)KS_ACTION_FEATURE_WORD_MAX_CHARACTERS;
        const cp* raw = whole.s;
        cp text[MAXS];
        for (int i = 0; i < rn; ++i) text[i] = casefold(M, raw[i]);
        characters(f, Ft, label, text, rn, direction);
        if (!word_score(f, Ft, M, label, side == 0 ? r.source : r.target, side == 0 ? r.source_frequency : r.target_frequency,
                        side == 0 ? r.source_flags : r.target_flags, side == 0 ? sk : tk)) return false;
        name.clear(); name.ascii(label); name.ascii(":identifier:"); name.integer(side == 0 ? si : ti); f.set(name, 1.0);
        cp letters[MAXS]; int ln = 0;
        for (int i = 0; i < rn; ++i) if (is_alpha(M, raw[i])) letters[ln++] = raw[i];
        const char* c = ln == 0 ? "none" : str_isupper(M, letters, ln) ? "upper" : str_islower(M, letters, ln) ? "lower"
                       : str_istitle(M, letters, ln) ? "title" : "mixed";
        if (side == 0) source_case = c;
        name.clear(); name.ascii(label); name.ascii(":case:"); name.ascii(c); f.set(name, 1.0);
        name.clear(); name.ascii(label); name.ascii(":script:"); name.ascii(script_of(text, rn)); f.set(name, 1.0);
        name.clear(); name.ascii(label); name.ascii(":length"); f.set(name, (double)rn / (double)KS_ACTION_FEATURE_WORD_MAX_CHARACTERS);
        int digits = 0;
        for (int i = 0; i < rn; ++i) digits += is_digit(M, raw[i]);
        double denominator = (double)(rn > 1 ? rn : 1);
        name.clear(); name.ascii(label); name.ascii(":digits"); f.set(name, (double)digits / denominator);
        for (int m = 0; m < 17; ++m) {
            cp mark = m == 6 ? 0x2019u : (cp)(unsigned char)marks[m < 6 ? m : m - 1];
            int count = 0;
            for (int i = 0; i < rn; ++i) count += raw[i] == mark;
            name.clear(); name.ascii(label); name.ascii(":mark:"); name.hex4(mark); f.set(name, (double)count / denominator);
        }
    }
    for (int side = 0; side < 2; ++side) {
        const char* label = side == 0 ? "before" : "after";
        const cp* raw = side == 0 ? row.before.s + bstart : row.after.s;
        int rn = side == 0 ? bn : an;
        cp text[MAXCTX];
        for (int i = 0; i < rn; ++i) text[i] = casefold(M, raw[i]);
        const char* script = script_of(text, rn);
        name.clear(); name.ascii(label); name.ascii(":script:"); name.ascii(script); name.ascii(":direction:"); name.ascii(direction); f.set(name, 1.0);
        name.clear(); name.ascii(label); name.ascii(":script:"); name.ascii(script); name.ascii(":direction:"); name.ascii(direction);
        name.ascii(":length:"); name.integer(length); f.set(name, 1.0);
        int starts[MAXCTX], ends[MAXCTX], words = 0;
        for (int i = 0; i < rn;) {
            if (word_letter(M, text[i])) { int end = word_end(M, text, rn, i); starts[words] = i; ends[words] = end; ++words; i = end; }
            else ++i;
        }
        int first = side == 0 ? (words > KS_ACTION_FEATURE_NEIGHBOUR_WORD_COUNT ? words - (int)KS_ACTION_FEATURE_NEIGHBOUR_WORD_COUNT : 0) : 0;
        int last = side == 0 ? words : (words < KS_ACTION_FEATURE_NEIGHBOUR_WORD_COUNT ? words : (int)KS_ACTION_FEATURE_NEIGHBOUR_WORD_COUNT);
        for (int w = first; w < last; ++w) {
            int wn = ends[w] - starts[w];
            if (wn > KS_ACTION_FEATURE_NEIGHBOUR_WORD_MAX_CHARACTERS) wn = (int)KS_ACTION_FEATURE_NEIGHBOUR_WORD_MAX_CHARACTERS;
            characters(f, Ft, label, text + starts[w], wn, direction);
        }
        int ws, we;
        if (side == 0) { we = rn; ws = rn; while (ws > 0 && is_space(M, raw[ws - 1])) --ws; }
        else { ws = 0; we = 0; while (we < rn && is_space(M, raw[we])) ++we; }
        if (we - ws > KS_ACTION_FEATURE_WHITESPACE_MAX_CHARACTERS) we = ws + (int)KS_ACTION_FEATURE_WHITESPACE_MAX_CHARACTERS;
        const double W = (double)KS_ACTION_FEATURE_WHITESPACE_MAX_CHARACTERS;
        name.clear(); name.ascii(label); name.ascii(":space_count"); f.set(name, (double)(we - ws) / W);
        cp previous = 0; bool any = false;
        while (true) {               // sorted(set(whitespace))
            cp next = 0xFFFFFFFFu;
            for (int i = ws; i < we; ++i) if ((!any || raw[i] > previous) && raw[i] < next) next = raw[i];
            if (next == 0xFFFFFFFFu) break;
            int count = 0;
            for (int i = ws; i < we; ++i) count += raw[i] == next;
            name.clear(); name.ascii(label); name.ascii(":space:"); name.hex4(next); f.set(name, (double)count / W);
            previous = next; any = true;
        }
    }
    const double S = (double)KS_ACTION_FEATURE_SHORT_TEXT_CHARACTERS;
    for (int side = 0; side < 2; ++side) {
        const Text& whole = side == 0 ? row.tail : row.boundary;
        const char* label = side == 0 ? "tail" : "boundary";
        int tn = whole.n < KS_ACTION_FEATURE_SHORT_TEXT_CHARACTERS ? whole.n : (int)KS_ACTION_FEATURE_SHORT_TEXT_CHARACTERS;
        name.clear(); name.ascii(label); name.ascii(":length"); f.set(name, (double)tn / S);
        name.clear(); name.ascii(label); name.ascii(":overflow"); f.set(name, whole.n > KS_ACTION_FEATURE_SHORT_TEXT_CHARACTERS ? 1.0 : 0.0);
        if (side == 1) {
            bool isspace = tn > 0;
            for (int i = 0; i < tn; ++i) if (!is_space(M, whole.s[i])) isspace = false;
            name.clear(); name.ascii("boundary:isspace"); f.set(name, isspace ? 1.0 : 0.0);
        }
        cp previous = 0; bool any = false;
        while (true) {
            cp next = 0xFFFFFFFFu;
            for (int i = 0; i < tn; ++i) if ((!any || whole.s[i] > previous) && whole.s[i] < next) next = whole.s[i];
            if (next == 0xFFFFFFFFu) break;
            int count = 0;
            for (int i = 0; i < tn; ++i) count += whole.s[i] == next;
            name.clear(); name.ascii(label); name.ascii(":char:"); name.hex4(next); f.set(name, (double)count / S);
            previous = next; any = true;
        }
    }
    int pn = row.application.n < KS_ACTION_FEATURE_APPLICATION_NAME_CHARACTERS ? row.application.n : (int)KS_ACTION_FEATURE_APPLICATION_NAME_CHARACTERS;
    cp application[MAXCTX];
    for (int i = 0; i < pn; ++i) application[i] = casefold(M, row.application.s[i]);
    int parts = 0;
    for (int i = 0; i < pn && parts < KS_ACTION_FEATURE_APPLICATION_TOKEN_COUNT;) {
        cp c = application[i];
        if ((c >= 'a' && c <= 'z') || (c >= '0' && c <= '9')) {
            int j = i;
            while (j < pn && ((application[j] >= 'a' && application[j] <= 'z') || (application[j] >= '0' && application[j] <= '9'))) ++j;
            name.clear(); name.ascii("app:"); name.text(application + i, j - i); f.set(name, 1.0);
            ++parts; i = j;
        } else ++i;
    }
    if (r.flags & RF_PROBABILITY) {
        double x; if (!bounded(r.probability, 0.0, 1.0, &x)) return false;
        ascii_name(name, "baseline:probability"); f.set(name, x);
        ascii_name(name, "baseline:probability:present"); f.set(name, 1.0);
    }
    if (r.flags & RF_THRESHOLD) {
        double x; if (!bounded(r.threshold, 0.0, 1.0, &x)) return false;
        ascii_name(name, "baseline:threshold"); f.set(name, x);
        ascii_name(name, "baseline:threshold:present"); f.set(name, 1.0);
    }
    if ((r.flags & RF_PROBABILITY) && (r.flags & RF_THRESHOLD)) {
        double x; if (!bounded(r.probability - r.threshold, -1.0, 1.0, &x)) return false;
        ascii_name(name, "baseline:margin"); f.set(name, x);
    }
    const double OB = KS_ACTION_FEATURE_ORTHO_SCORE_BOUND;
    if (r.flags & RF_ORTHO_SCORE) {
        double x; if (!bounded(r.ortho_score, -OB, OB, &x)) return false;
        ascii_name(name, "ortho:score"); f.set(name, x / OB);
        ascii_name(name, "ortho:score:present"); f.set(name, 1.0);
    }
    if (r.flags & RF_ORTHO_THRESHOLD) {
        double x; if (!bounded(r.ortho_threshold, -OB, OB, &x)) return false;
        ascii_name(name, "ortho:threshold"); f.set(name, x / OB);
        ascii_name(name, "ortho:threshold:present"); f.set(name, 1.0);
    }
    if ((r.flags & RF_ORTHO_SCORE) && (r.flags & RF_ORTHO_THRESHOLD)) {
        double x; if (!bounded(r.ortho_score - r.ortho_threshold, -OB, OB, &x)) return false;
        double margin = x / OB;
        ascii_name(name, "ortho:margin"); f.set(name, margin);
        const char* side = margin > 0 ? "positive" : margin < 0 ? "negative" : "zero";
        name.clear(); name.ascii("ortho:side:"); name.ascii(side); name.ascii(":direction:"); name.ascii(direction);
        name.ascii(":source_known:"); name.integer(sk); f.set(name, 1.0);
        name.clear(); name.ascii("ortho:side:"); name.ascii(side); name.ascii(":direction:"); name.ascii(direction);
        name.ascii(":source_known:"); name.integer(sk); name.ascii(":case:"); name.ascii(source_case); f.set(name, 1.0);
        const char* context_script = script_of(before_folded, bn);
        for (int s = 0; s < 2; ++s) {
            const char* sign = s == 0 ? "positive" : "negative";
            double value = s == 0 ? py_max(0.0, margin) : py_min(0.0, margin);
            name.clear(); name.ascii("ortho:"); name.ascii(sign); name.ascii(":direction:"); name.ascii(direction); f.set(name, value);
            name.clear(); name.ascii("ortho:"); name.ascii(sign); name.ascii(":direction:"); name.ascii(direction);
            name.ascii(":source_known:"); name.integer(sk); name.ascii(":context:"); name.ascii(context_script); f.set(name, value);
        }
    }
    term_features(f, M, Ft, row, r, direction);
    return !f.overflow;
}
