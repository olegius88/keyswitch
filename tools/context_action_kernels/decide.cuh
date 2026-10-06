// CUDA featurizer, part 2: the trainer's previous context and automatic_word_decision with the
// detector, the intent model and the short-word rules (detector.py, word_decision.py, short_words.py,
// intent_model.py). A decision is kept as the few fields the evidence reads.
#pragma once
#include "core.cuh"

struct Intent {
    u64 weights;             // int16 per bucket
    u64 dimension_mask;      // dimension - 1
    u64 fnv_seed;
    double weight_scale, bias;
    double platt_scale[KS_LAYOUT_GROUP_COUNT], platt_bias[KS_LAYOUT_GROUP_COUNT];          // per direction 0>1, 1>0
    double threshold_logit[KS_CONTEXT_ACTION_CUDA_TRIGGER_CAPACITY][KS_LAYOUT_GROUP_COUNT], threshold[KS_CONTEXT_ACTION_CUDA_TRIGGER_CAPACITY][KS_LAYOUT_GROUP_COUNT]; // per trigger index and direction
};

struct Decide {
    u64 translate[KS_LAYOUT_GROUP_COUNT];        // context_physical_keys: per group, code point -> other group's, 0 = none
    HashTable protected_tokens;
    HashTable trusted_short_table[KS_LAYOUT_GROUP_COUNT];
    HashTable single_letter; // TRUSTED_SINGLE_LETTER_WORDS
    Intent intent;
};

struct Row {                 // one ActionRow as the GPU reads it
    const cp* original; int original_n; int group;
    const cp* before; int before_n; const cp* after; int after_n;
    const cp* application; int application_n;
    const cp* trigger; int trigger_n; int trigger_index;
    int planned;             // after_origin == "planned_next_conversion"
};

enum Reason { R_OTHER = 0, R_EXACT_ONLY = 1, R_MORPH_CONFIRMED = 2 };

struct Decision {            // what evidence_for_decision and the vetoes read of a DetectionDecision
    bool should_convert; int reason; int target_group;
    bool has_prediction; double calibrated; double threshold;
    double source_ngram;     // decision.source_score.ngram_score (natural_short_source_veto)
    const cp* replacement; int replacement_n;
};

// context_physical_keys.translated; false when a glyph has no key in `group`.
__device__ bool physical_translate(const Model* M, const Decide* Dc, const cp* s, int n, int group, cp* out) {
    const unsigned* table = (const unsigned*)Dc->translate[group];
    for (int i = 0; i < n; ++i) {
        unsigned other = s[i] < 65536u ? table[s[i]] : 0u;
        if (other == 0u) return false;
        out[i] = other;
    }
    return true;
}

// The trainer's WORDS = [A-Za-zА-Яа-яЁё]+(?:['’\-][A-Za-zА-Яа-яЁё]+)*
__device__ __forceinline__ bool words_letter(cp c) {
    return (c >= 'A' && c <= 'Z') || (c >= 'a' && c <= 'z') || (c >= 0x410u && c <= 0x44Fu) || c == 0x401u || c == 0x451u;
}
__device__ __forceinline__ bool words_joiner(cp c) { return c == '\'' || c == 0x2019u || c == '-'; }
// End of the WORDS match starting at `i` (a letter), greedy as re matches it.
__device__ int words_match_end(const cp* s, int n, int i) {
    int j = i;
    while (j < n && words_letter(s[j])) ++j;
    while (j + 1 < n && words_joiner(s[j]) && words_letter(s[j + 1])) {
        j += 1;
        while (j < n && words_letter(s[j])) ++j;
    }
    return j;
}

__device__ bool str_has_nonspace(const Model* M, const cp* s, int n) {
    for (int i = 0; i < n; ++i) if (!is_space(M, s[i])) return true;
    return false;
}

// Previous words of train_context_action_model.previous_context (and of a planned frame).
struct Previous { cp words[KS_LAYOUT_GROUP_COUNT][MAXCTX]; int lengths[KS_LAYOUT_GROUP_COUNT]; bool present[KS_LAYOUT_GROUP_COUNT]; int context_group; bool valid; };

__device__ void previous_context(const Model* M, const Decide* Dc, const Row& r, Previous* p, RowState* st) {
    p->present[0] = p->present[1] = false; p->lengths[0] = p->lengths[1] = 0;
    p->context_group = -1;
    if (r.planned) {
        int i = 0;
        while (i < r.after_n && is_space(M, r.after[i])) ++i;
        if (i >= r.after_n || !words_letter(r.after[i])) { st->fallback = true; return; }  // Python raises
        int end = words_match_end(r.after, r.after_n, i);
        int g = 1 - r.group;
        if (end - i > MAXCTX) { st->fallback = true; return; }
        for (int k = i; k < end; ++k) p->words[g][k - i] = r.after[k];
        p->lengths[g] = end - i; p->present[g] = true; p->context_group = g;
        return;
    }
    int start = -1, stop = -1;
    for (int i = 0; i < r.before_n;) {
        if (words_letter(r.before[i])) { int end = words_match_end(r.before, r.before_n, i); start = i; stop = end; i = end; }
        else ++i;
    }
    if (start < 0 || !str_has_nonspace(M, r.application, r.application_n)) return;
    int n = stop - start;
    if (n > MAXCTX) { st->fallback = true; return; }
    int group = 0;
    for (int k = start; k < stop; ++k) {
        cp c = casefold(M, r.before[k]);
        if ((c >= 0x430u && c <= 0x44Fu) || c == 0x451u) { group = 1; break; }
    }
    cp alternative[MAXCTX];
    if (!physical_translate(M, Dc, r.before + start, n, group, alternative)) return;
    for (int k = 0; k < n; ++k) { p->words[group][k] = r.before[start + k]; p->words[1 - group][k] = alternative[k]; }
    p->lengths[0] = p->lengths[1] = n; p->present[0] = p->present[1] = true;
    p->context_group = group;
}

// LanguageModel.context_score
__device__ double context_score(const Model* M, int locale, const cp* previous, int previous_n, const cp* word, int n) {
    cp key[2 * MAXS + 1];
    int a = lm_normalize(M, previous, previous_n, key);
    key[a] = 0u;
    int b = lm_normalize(M, word, n, key + a + 1);
    int index = table_find(M->bigram[locale], key, a + 1 + b);
    return index >= 0 ? ((const double*)M->bigram_value[locale])[index] : 0.0;
}

// LanguageDetector._context_delta
__device__ double context_delta(const Model* M, int source_group, int target_group, const cp* source_word, int sn,
                                const cp* target_word, int tn, const Previous* p) {
    double source_context = context_score(M, source_group, p->words[source_group], p->present[source_group] ? p->lengths[source_group] : 0, source_word, sn);
    double target_context = context_score(M, target_group, p->words[target_group], p->present[target_group] ? p->lengths[target_group] : 0, target_word, tn);
    double delta = KS_CONTEXT_DELTA_MULTIPLIER * (target_context - source_context);
    if (p->context_group == target_group) delta += KS_CONTEXT_TARGET_GROUP_BONUS;
    else if (p->context_group == source_group) delta -= KS_CONTEXT_SOURCE_GROUP_PENALTY;
    return delta;
}

// LanguageDetector._looks_like_protected_token
__device__ bool looks_protected(const Model* M, const Decide* Dc, const cp* token, int n) {
    if (n > KS_MAX_CONVERTIBLE_TOKEN_CHARACTERS) return true;
    cp lowered[MAXS];
    for (int i = 0; i < n; ++i) lowered[i] = casefold(M, token[i]);
    if (table_find(Dc->protected_tokens, lowered, n) >= 0) return true;
    if (n > 0 && lowered[0] == '-') return true;
    const char* markers[4] = {"http://", "https://", "www.", "@"};
    for (int m = 0; m < 4; ++m) {
        int length = 0; while (markers[m][length]) ++length;
        for (int i = 0; i + length <= n; ++i) {
            bool same = true;
            for (int k = 0; k < length; ++k) if (lowered[i + k] != (cp)markers[m][k]) { same = false; break; }
            if (same) return true;
        }
    }
    for (int i = 0; i < n; ++i) if (is_digit(M, token[i])) return true;
    for (int i = 0; i < n; ++i) {
        cp c = token[i];
        if (c == '_' || c == '/' || c == '\\' || c == '=' || c == ':') return true;
    }
    int letters = 0; bool all_upper = true, later_upper = false, cyrillic = false, latin = false;
    for (int i = 0; i < n; ++i) {
        unsigned p = props(M, token[i]);
        if (!(p & P_ALPHA)) continue;
        if (!(p & P_UPPER)) all_upper = false;
        if (letters > 0 && (p & P_UPPER)) later_upper = true;
        if (p & P_CYRILLIC_NAME) cyrillic = true; else if (p & P_LATIN_NAME) latin = true;
        ++letters;
    }
    if (letters >= KS_ACRONYM_MIN_LETTERS && all_upper) return true;
    if (later_upper) return true;
    const int run = KS_REPEATED_CHARACTER_RUN;
    for (int i = 0; i < n - (run - 1); ++i) {
        bool repeated = true;
        for (int k = 1; k < run; ++k) if (lowered[i + k] != lowered[i]) { repeated = false; break; }
        if (repeated) return true;
    }
    return cyrillic && latin;
}

// LanguageModel.best_single_deletion
__device__ WordScore best_single_deletion(const Model* M, int locale, bool spelling, const cp* word, int n, RowState* st) {
    if (n < KS_LANGUAGE_MODEL_DELETION_MIN_CHARACTERS) return lm_score(M, locale, spelling, word, 0, st);
    const int limit = KS_LANGUAGE_MODEL_DELETION_POSITIONS;
    int indices[KS_LANGUAGE_MODEL_DELETION_POSITIONS]; int count = 0;
    if (n > limit) {
        for (int index = 0; index < limit; ++index) {
            int value = (int)rint((double)(index * (n - 1)) / (double)(limit - 1));
            bool seen = false;
            for (int k = 0; k < count; ++k) if (indices[k] == value) { seen = true; break; }
            if (!seen) indices[count++] = value;
        }
        for (int a = 1; a < count; ++a) {       // sorted(set(...))
            int v = indices[a], b = a - 1;
            while (b >= 0 && indices[b] > v) { indices[b + 1] = indices[b]; --b; }
            indices[b + 1] = v;
        }
    } else { for (int i = 0; i < n; ++i) indices[count++] = i; }
    WordScore best; bool have = false;
    cp shorter[MAXS];
    for (int k = 0; k < count; ++k) {
        int index = indices[k], m = 0;
        for (int i = 0; i < n; ++i) if (i != index) shorter[m++] = word[i];
        WordScore s = lm_score(M, locale, spelling, shorter, m, st);
        if (!have || s.value > best.value) { best = s; have = true; }
    }
    return best;
}

// ---- intent_model.extract_features / LinearNgramModel.predict ----
__device__ __forceinline__ u64 fnv_byte(u64 h, unsigned char b) { h ^= (u64)b; h *= (u64)KS_FNV1A64_PRIME; return h; }
__device__ u64 fnv_utf8(u64 h, cp c) {
    if (c < 0x80u) return fnv_byte(h, (unsigned char)c);
    if (c < 0x800u) { h = fnv_byte(h, 0xC0u | (c >> 6)); return fnv_byte(h, 0x80u | (c & 0x3Fu)); }
    if (c < 0x10000u) { h = fnv_byte(h, 0xE0u | (c >> 12)); h = fnv_byte(h, 0x80u | ((c >> 6) & 0x3Fu)); return fnv_byte(h, 0x80u | (c & 0x3Fu)); }
    h = fnv_byte(h, 0xF0u | (c >> 18)); h = fnv_byte(h, 0x80u | ((c >> 12) & 0x3Fu));
    h = fnv_byte(h, 0x80u | ((c >> 6) & 0x3Fu)); return fnv_byte(h, 0x80u | (c & 0x3Fu));
}
__device__ u64 fnv_ascii(u64 h, const char* text) { for (; *text; ++text) h = fnv_byte(h, (unsigned char)*text); return h; }
__device__ u64 fnv_int(u64 h, long long value) {
    char digits[KS_CONTEXT_ACTION_CUDA_DECIMAL_DIGITS]; int n = 0; bool negative = value < 0; unsigned long long v = negative ? (unsigned long long)(-value) : (unsigned long long)value;
    do { digits[n++] = (char)('0' + v % 10); v /= 10; } while (v);
    if (negative) h = fnv_byte(h, '-');
    while (n) h = fnv_byte(h, (unsigned char)digits[--n]);
    return h;
}

// The n-grams of two bounded tokens and the five dense features fit the entries, and the slots outnumber them.
static_assert(2 * KS_INTENT_NGRAM_ORDERS_COUNT * (KS_INTENT_RAW_TOKEN_MAX_CHARACTERS + 2) + 8 <= KS_CONTEXT_ACTION_CUDA_INTENT_ENTRIES,
              "intent entries too few for two tokens");
static_assert(KS_CONTEXT_ACTION_CUDA_INTENT_ENTRIES < (1 << KS_CONTEXT_ACTION_CUDA_INTENT_SLOT_BITS), "intent slots too few");
static_assert(KS_INTENT_RAW_TOKEN_MAX_CHARACTERS + 2 < (1 << KS_CONTEXT_ACTION_CUDA_NGRAM_SLOT_BITS), "n-gram slots too few");
#define INTENT_SLOTS (1 << KS_CONTEXT_ACTION_CUDA_INTENT_SLOT_BITS)
#define NGRAM_SLOTS (1 << KS_CONTEXT_ACTION_CUDA_NGRAM_SLOT_BITS)
struct IntentFeatures {      // values[bucket] accumulated in call order (Python dict semantics)
    int buckets[KS_CONTEXT_ACTION_CUDA_INTENT_ENTRIES]; double values[KS_CONTEXT_ACTION_CUDA_INTENT_ENTRIES];
    short slots[INTENT_SLOTS]; int n;
    __device__ void clear() { n = 0; for (int i = 0; i < INTENT_SLOTS; ++i) slots[i] = -1; }
    __device__ void add(const Intent& I, u64 hashed, double value) {
        if (value == 0.0) return;
        int bucket = (int)(hashed & I.dimension_mask);
        double sign = (hashed & KS_FEATURE_HASH_SIGN_BIT) ? -1.0 : 1.0;
        unsigned slot = ((unsigned)bucket * (unsigned)KS_CONTEXT_ACTION_CUDA_MULTIPLICATIVE_HASH) >> (32 - KS_CONTEXT_ACTION_CUDA_INTENT_SLOT_BITS);
        while (slots[slot] >= 0) {
            int i = slots[slot];
            if (buckets[i] == bucket) { values[i] = values[i] + value * sign; return; }
            slot = (slot + 1) & (INTENT_SLOTS - 1);
        }
        slots[slot] = (short)n; buckets[n] = bucket; values[n] = 0.0 + value * sign; ++n;
    }
};

// intent_model._add_character_features: Counter of the n-grams of ^token$ in first-occurrence order.
__device__ void intent_characters(const Intent& I, IntentFeatures* f, const cp* token, int tn, int group, double polarity) {
    cp bounded[KS_INTENT_RAW_TOKEN_MAX_CHARACTERS + 2];
    bounded[0] = '^';
    for (int i = 0; i < tn; ++i) bounded[i + 1] = token[i];
    bounded[tn + 1] = '$';
    int length = tn + 2;
    for (int o = 0; o < KS_INTENT_NGRAM_ORDERS_COUNT; ++o) {
        int order = (int)KS_INTENT_NGRAM_ORDERS[o];
        if (length < order) continue;
        int count = length - order + 1;
        int firsts[KS_INTENT_RAW_TOKEN_MAX_CHARACTERS + 2], counts[KS_INTENT_RAW_TOKEN_MAX_CHARACTERS + 2], unique = 0;
        short slots[NGRAM_SLOTS];
        for (int i = 0; i < NGRAM_SLOTS; ++i) slots[i] = -1;
        for (int i = 0; i < count; ++i) {
            unsigned slot = (unsigned)(fnv_cp(bounded + i, order) >> (64 - KS_CONTEXT_ACTION_CUDA_NGRAM_SLOT_BITS));
            int found = -1;
            while (slots[slot] >= 0) {
                int u = slots[slot];
                if (str_eq(bounded + firsts[u], order, bounded + i, order)) { found = u; break; }
                slot = (slot + 1) & (NGRAM_SLOTS - 1);
            }
            if (found >= 0) counts[found] += 1;
            else { slots[slot] = (short)unique; firsts[unique] = i; counts[unique] = 1; ++unique; }
        }
        long long squares = 0;
        for (int u = 0; u < unique; ++u) squares += (long long)counts[u] * counts[u];
        double norm = sqrt((double)squares);
        u64 prefix = fnv_ascii(I.fnv_seed, "char:g");
        prefix = fnv_int(prefix, group);
        prefix = fnv_ascii(prefix, ":n");
        prefix = fnv_int(prefix, order);
        prefix = fnv_byte(prefix, ':');
        for (int u = 0; u < unique; ++u) {
            u64 h = prefix;
            for (int k = 0; k < order; ++k) h = fnv_utf8(h, bounded[firsts[u] + k]);
            f->add(I, h, polarity * (double)counts[u] / norm);
        }
    }
}

__device__ u64 fnv_length_bucket(u64 h, int length) {
    if (length <= KS_INTENT_LENGTH_EXACT_MAX_CHARACTERS) return fnv_int(h, length);
    if (length <= KS_INTENT_LENGTH_SHORT_MAX_CHARACTERS) {
        h = fnv_int(h, KS_INTENT_LENGTH_EXACT_MAX_CHARACTERS + 1); h = fnv_byte(h, '-'); return fnv_int(h, KS_INTENT_LENGTH_SHORT_MAX_CHARACTERS); }
    if (length <= KS_INTENT_LENGTH_MEDIUM_MAX_CHARACTERS) {
        h = fnv_int(h, KS_INTENT_LENGTH_SHORT_MAX_CHARACTERS + 1); h = fnv_byte(h, '-'); return fnv_int(h, KS_INTENT_LENGTH_MEDIUM_MAX_CHARACTERS); }
    if (length <= KS_INTENT_LENGTH_LONG_MAX_CHARACTERS) {
        h = fnv_int(h, KS_INTENT_LENGTH_MEDIUM_MAX_CHARACTERS + 1); h = fnv_byte(h, '-'); return fnv_int(h, KS_INTENT_LENGTH_LONG_MAX_CHARACTERS); }
    h = fnv_int(h, KS_INTENT_LENGTH_LONG_MAX_CHARACTERS + 1); return fnv_byte(h, '+');
}

// intent_model.normalize_token on the fast path: bound, casefold (NFC leaves these characters alone).
__device__ int intent_normalize(const Model* M, const cp* s, int n, cp* out) {
    int k = n < KS_INTENT_RAW_TOKEN_MAX_CHARACTERS ? n : (int)KS_INTENT_RAW_TOKEN_MAX_CHARACTERS;
    for (int i = 0; i < k; ++i) out[i] = casefold(M, s[i]);
    return k;
}

// LinearNgramModel.predict, without the probability (exp runs on the host): calibrated logit,
// the decision and the threshold.
__device__ void intent_predict(const Model* M, const Intent& I, const cp* original, int on, const cp* alternative, int an,
                               int source_group, int target_group, const cp* trigger, int trigger_n, int trigger_index,
                               bool* should_switch, double* calibrated_out, double* threshold_out) {
    cp a[KS_INTENT_RAW_TOKEN_MAX_CHARACTERS], b[KS_INTENT_RAW_TOKEN_MAX_CHARACTERS];
    int na = intent_normalize(M, original, on, a), nb = intent_normalize(M, alternative, an, b);
    int active = 0;
    for (int o = 0; o < KS_INTENT_NGRAM_ORDERS_COUNT; ++o) if (na + 2 >= KS_INTENT_NGRAM_ORDERS[o] || nb + 2 >= KS_INTENT_NGRAM_ORDERS[o]) ++active;
    double scale = active ? 1.0 / sqrt((double)active) : 1.0;
    IntentFeatures f; f.clear();
    int sg = source_group < KS_INTENT_MAX_FEATURE_GROUP ? source_group : (int)KS_INTENT_MAX_FEATURE_GROUP;
    int tg = target_group < KS_INTENT_MAX_FEATURE_GROUP ? target_group : (int)KS_INTENT_MAX_FEATURE_GROUP;
    intent_characters(I, &f, a, na, sg, -scale);
    intent_characters(I, &f, b, nb, tg, scale);
    int longest = na > nb ? na : nb;
    f.add(I, fnv_ascii(I.fnv_seed, "dense:length_delta"), (double)(nb - na) / (double)KS_INTENT_RAW_TOKEN_MAX_CHARACTERS);
    u64 h = fnv_ascii(I.fnv_seed, "interaction:direction:");
    h = fnv_int(h, sg); h = fnv_byte(h, '>'); h = fnv_int(h, tg); h = fnv_ascii(h, ":length:"); h = fnv_length_bucket(h, longest);
    f.add(I, h, 1.0);
    h = fnv_ascii(I.fnv_seed, "interaction:trigger:");
    for (int i = 0; i < trigger_n; ++i) h = fnv_utf8(h, trigger[i]);
    h = fnv_ascii(h, ":length:"); h = fnv_length_bucket(h, longest);
    f.add(I, h, 1.0);
    h = fnv_ascii(I.fnv_seed, "length:"); h = fnv_length_bucket(h, longest); f.add(I, h, 1.0);
    h = fnv_ascii(I.fnv_seed, "direction:"); h = fnv_int(h, sg); h = fnv_byte(h, '>'); h = fnv_int(h, tg); f.add(I, h, 1.0);
    h = fnv_ascii(I.fnv_seed, "trigger:");
    for (int i = 0; i < trigger_n; ++i) h = fnv_utf8(h, trigger[i]);
    f.add(I, h, 1.0);
    // sorted(values.items()), nonzero values only; shell sort by bucket
    for (int gap = f.n / 2; gap > 0; gap /= 2)
        for (int i = gap; i < f.n; ++i) {
            int bucket = f.buckets[i]; double value = f.values[i]; int j = i;
            while (j >= gap && f.buckets[j - gap] > bucket) { f.buckets[j] = f.buckets[j - gap]; f.values[j] = f.values[j - gap]; j -= gap; }
            f.buckets[j] = bucket; f.values[j] = value;
        }
    const short* weights = (const short*)I.weights;
    double logit = I.bias;
    for (int i = 0; i < f.n; ++i) {
        if (f.values[i] == 0.0) continue;
        logit += (double)weights[f.buckets[i]] * I.weight_scale * f.values[i];
    }
    int direction = source_group == 0 ? 0 : 1;
    double calibrated = (I.platt_scale[direction] * logit) + I.platt_bias[direction];
    *calibrated_out = calibrated;
    *threshold_out = I.threshold[trigger_index][direction];
    *should_switch = calibrated >= I.threshold_logit[trigger_index][direction];
}

// ---- short_words / word_decision ----
__device__ bool in_table(const HashTable& t, const cp* s, int n) { return table_find(t, s, n) >= 0; }

// word_decision.automatic_word_decision (detector.decide with the defaults the trainer passes,
// natural_short_source_veto, word_shape_veto, trusted_short_word_decision, opening_letter_decision).
__device__ Decision automatic_word_decision(const Model* M, const Decide* Dc, const Row& r, bool spelling,
                                            const cp* alternative, int an, const Previous* prev, RowState* st) {
    const int sg = r.group, tg = 1 - r.group;
    const cp* original = r.original; const int on = r.original_n;
    const double threshold = KS_DEFAULT_CONFIDENCE_THRESHOLD;
    WordScore source_score = lm_score(M, sg, spelling, original, on, st);
    Decision d;
    d.should_convert = false; d.reason = R_OTHER; d.target_group = sg; d.has_prediction = false;
    d.calibrated = 0.0; d.threshold = 0.0; d.source_ngram = source_score.ngram_score;
    d.replacement = original; d.replacement_n = on;
    bool candidate = !str_eq(alternative, an, original, on);
    WordScore target_score;
    bool have_target = false;
    cp scratch[MAXS];
    if (candidate) {
        target_score = lm_score(M, tg, spelling, alternative, an, st);
        have_target = true;
        double delta = target_score.value - source_score.value;
        delta += context_delta(M, sg, tg, original, on, alternative, an, prev);
        int effective = lm_normalize(M, original, on, scratch);
        int other = lm_normalize(M, alternative, an, scratch);
        if (other > effective) effective = other;
        bool rejected = effective < KS_DEFAULT_MINIMUM_WORD_LENGTH || looks_protected(M, Dc, original, on) || source_score.known;
        if (!rejected) {
            d.replacement = alternative; d.replacement_n = an; d.target_group = tg;
            int lo = on < KS_INTENT_RAW_TOKEN_MAX_CHARACTERS ? on : (int)KS_INTENT_RAW_TOKEN_MAX_CHARACTERS;
            int la = an < KS_INTENT_RAW_TOKEN_MAX_CHARACTERS ? an : (int)KS_INTENT_RAW_TOKEN_MAX_CHARACTERS;
            if ((lo > la ? lo : la) >= KS_INTENT_MIN_RUNTIME_TOKEN_CHARACTERS) {
                bool should_switch;
                intent_predict(M, Dc->intent, original, on, alternative, an, sg, tg, r.trigger, r.trigger_n, r.trigger_index,
                               &should_switch, &d.calibrated, &d.threshold);
                d.has_prediction = true;
                d.should_convert = should_switch;
            } else if (target_score.known) {
                double relief = py_min(1.0, (double)(effective - KS_HEURISTIC_KNOWN_TARGET_RELIEF_START_CHARACTERS > 0
                                                     ? effective - KS_HEURISTIC_KNOWN_TARGET_RELIEF_START_CHARACTERS : 0)
                                           * KS_HEURISTIC_KNOWN_TARGET_RELIEF_PER_CHARACTER);
                double required = py_max(KS_HEURISTIC_KNOWN_TARGET_MIN_MARGIN, threshold - relief);
                if (target_score.spell_known && !target_score.exact) required += KS_HEURISTIC_SPELL_ONLY_TARGET_EXTRA_MARGIN;
                bool plausible = target_score.ngram_score >= KS_LANGUAGE_MODEL_KNOWN_WORD_NATURALNESS_FLOOR
                                 || prev->context_group == tg;
                d.should_convert = delta >= required && plausible;
                d.reason = target_score.exact ? R_EXACT_ONLY : d.should_convert ? R_MORPH_CONFIRMED : R_OTHER;
            } else {
                bool done = false;
                if (effective >= KS_HEURISTIC_TYPO_DELETION_MIN_CHARACTERS) {
                    WordScore target_without = best_single_deletion(M, tg, spelling, alternative, an, st);
                    WordScore source_without = best_single_deletion(M, sg, spelling, original, on, st);
                    double typo_delta = target_without.value - py_max(source_score.value, source_without.value);
                    if (target_without.known && !source_without.known
                        && typo_delta >= threshold + KS_HEURISTIC_TYPO_DELETION_EXTRA_MARGIN) {
                        d.should_convert = true; done = true;
                    }
                }
                if (!done) {
                    double relief = py_min(KS_HEURISTIC_NGRAM_RELIEF_CAP,
                        (double)(effective - KS_HEURISTIC_NGRAM_RELIEF_START_CHARACTERS > 0
                                 ? effective - KS_HEURISTIC_NGRAM_RELIEF_START_CHARACTERS : 0) * KS_HEURISTIC_NGRAM_RELIEF_PER_CHARACTER);
                    double required = threshold + KS_HEURISTIC_NGRAM_EXTRA_MARGIN - relief;
                    required = py_max(threshold + KS_HEURISTIC_NGRAM_MIN_EXTRA_MARGIN, required);
                    bool unlikely = source_score.ngram_score <= KS_HEURISTIC_UNLIKELY_SOURCE_NGRAM_MAX;
                    bool plausible = target_score.ngram_score >= KS_HEURISTIC_PLAUSIBLE_TARGET_NGRAM_MIN;
                    d.should_convert = effective >= KS_HEURISTIC_NGRAM_MIN_CHARACTERS && unlikely && plausible && delta >= required;
                }
            }
        }
    }
    // natural_short_source_veto
    if (d.should_convert && (d.reason == R_EXACT_ONLY || d.reason == R_MORPH_CONFIRMED)) {
        int length = lm_normalize(M, original, on, scratch);
        int other = lm_normalize(M, d.replacement, d.replacement_n, scratch);
        if (other > length) length = other;
        if (!(length > KS_NATURAL_SOURCE_MAX_LENGTH || d.source_ngram <= KS_HEURISTIC_UNLIKELY_SOURCE_NGRAM_MAX
              || prev->context_group == d.target_group)) { d.should_convert = false; d.reason = R_OTHER; }
    }
    // word_shape_veto
    if (d.should_convert && str_isalpha(M, original, on) && !str_isalpha(M, d.replacement, d.replacement_n)) {
        d.should_convert = false; d.reason = R_OTHER;
    }
    if (d.should_convert) return d;
    // trusted_short_word_decision
    if (!looks_protected(M, Dc, original, on) && candidate) {
        int no = lm_normalize(M, original, on, scratch);
        cp normalized[MAXS];
        int nr = lm_normalize(M, alternative, an, normalized);
        if ((no > nr ? no : nr) <= KS_TRUSTED_SHORT_WORD_MAX_LENGTH && in_table(Dc->trusted_short_table[tg], normalized, nr)) {
            bool supported = prev->context_group == tg;
            if (!have_target) { target_score = lm_score(M, tg, spelling, alternative, an, st); have_target = true; }
            bool take = false;
            if (in_table(Dc->single_letter, normalized, nr)) take = supported;
            else if (target_score.exact && target_score.frequency >= KS_TRUSTED_SHORT_WORD_MINIMUM_FREQUENCY) {
                double ratio = (double)(target_score.frequency + 1) / (double)(source_score.frequency + 1);
                take = !(ratio < KS_TRUSTED_SHORT_WORD_MINIMUM_RATIO && !(supported && ratio >= KS_TRUSTED_SHORT_WORD_CONTEXT_RATIO));
            }
            if (take) {
                Decision o = d;
                o.should_convert = true; o.reason = R_OTHER; o.target_group = tg; o.has_prediction = false;
                o.replacement = alternative; o.replacement_n = an;
                return o;
            }
        }
    }
    // opening_letter_decision (context tracked)
    if (prev->context_group < 0 && lm_normalize(M, original, on, scratch) == 1 && !looks_protected(M, Dc, original, on) && candidate) {
        cp normalized[MAXS];
        int nr = lm_normalize(M, alternative, an, normalized);
        if (in_table(Dc->single_letter, normalized, nr)) {
            Decision o = d;
            o.should_convert = true; o.reason = R_OTHER; o.target_group = tg; o.has_prediction = false;
            o.replacement = alternative; o.replacement_n = an;
            return o;
        }
    }
    return d;
}
