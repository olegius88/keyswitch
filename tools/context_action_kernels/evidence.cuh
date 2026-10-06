// CUDA featurizer, part 3: ortho_model.OrthoModel.score, context_model.one_typo_from_word and
// context_policy.evidence_for_decision as the trainer's evidence() calls it.
#pragma once
#include "rows.cuh"

struct Channel { HashTable logprob; u64 logprob_value; HashTable backoff; u64 backoff_value; double uniform; i64 present; };

enum OrthoFeature { F_IDENT = 0, F_REPEAT, F_PARTS_SOURCE, F_PARTS_TARGET, F_DOT_WORD, F_EXTRA_KEY, F_SOURCE_WORD,
                    F_NAME_UNKNOWN, F_TARGET_UNKNOWN, F_SOURCE_PLAUSIBILITY, F_TARGET_PLAUSIBILITY, F_COUNT };

struct Ortho {
    i64 order;
    Channel channels[KS_LAYOUT_GROUP_COUNT], source_channels[KS_LAYOUT_GROUP_COUNT];       // per script: 0 en, 1 ru
    double prose_shape[KS_LAYOUT_GROUP_COUNT][SHAPE_COUNT], acronym_shape[KS_LAYOUT_GROUP_COUNT][SHAPE_COUNT];  // SHAPE_* (ortho_model.SHAPES)
    double thresholds[KS_LAYOUT_GROUP_COUNT];
    i64 feature_count[KS_LAYOUT_GROUP_COUNT]; i64 feature_code[KS_LAYOUT_GROUP_COUNT][KS_CONTEXT_ACTION_CUDA_ORTHO_FEATURE_CAPACITY]; double feature_weight[KS_LAYOUT_GROUP_COUNT][KS_CONTEXT_ACTION_CUDA_ORTHO_FEATURE_CAPACITY];
    i64 has_centres; double centres[KS_LAYOUT_GROUP_COUNT]; i64 has_target_centres; double target_centres[KS_LAYOUT_GROUP_COUNT];
    i64 gated_count; i64 gated_code[KS_CONTEXT_ACTION_CUDA_ORTHO_FEATURE_CAPACITY];
    i64 compound_min[KS_LAYOUT_GROUP_COUNT]; i64 collapse; i64 stretch; i64 hyphens; i64 unscored_mask;
    u64 us_to_ru, ru_to_us;                         // LayoutPair.translate, per code point (0 = unchanged)
    HashTable identifiers;                          // packaged identifier lexicon
};

struct EvidenceTables {
    Ortho ortho;
    HashTable opening[KS_LAYOUT_GROUP_COUNT];                           // short_words.OPENING_WORDS
    u64 alphabet[KS_LAYOUT_GROUP_COUNT]; i64 alphabet_n[KS_LAYOUT_GROUP_COUNT];             // BOUNDARY_MISSING_LETTERS per locale
};

// The known() of the engine's dictionaries: LanguageModel.score(word).known with morphology.
__device__ bool lm_known(const Model* M, int locale, const cp* word, int n, RowState* st) {
    cp normalized[MAXS];
    int k = lm_normalize(M, word, n, normalized);
    if (k == 0) return false;
    bool eligible = true;
    for (int i = 0; i < n; ++i) { cp c = word[i]; if (!(is_alpha(M, c) || c == '\'' || c == '-')) { eligible = false; break; } }
    if (!eligible) return false;
    if (table_find(M->freq[locale], normalized, k) >= 0) return true;  // exact: the spell checker cannot change `known`
    return str_isalpha(M, normalized, k) ? hunspell_check(M, locale, normalized, k, st) : false;
}

__device__ void layout_translate(const Ortho* O, const cp* s, int n, bool us_to_ru, cp* out) {
    const unsigned* table = (const unsigned*)(us_to_ru ? O->us_to_ru : O->ru_to_us);
    for (int i = 0; i < n; ++i) { unsigned mapped = s[i] < 65536u ? table[s[i]] : 0u; out[i] = mapped ? mapped : s[i]; }
}

__device__ int collapse_runs(const cp* s, int n, int limit, cp* out) {
    int k = 0, run = 0;
    for (int i = 0; i < n; ++i) {
        run = (k > 0 && s[i] == out[k - 1]) ? run + 1 : 1;
        if (run <= limit) out[k++] = s[i];
    }
    return k;
}
__device__ int read_once(const cp* s, int n, int minimum, cp* out) {
    int k = 0, start = 0;
    for (int index = 1; index <= n; ++index) {
        if (index == n || s[index] != s[start]) {
            int length = index - start;
            if (length >= minimum) out[k++] = s[start];
            else for (int i = start; i < index; ++i) out[k++] = s[i];
            start = index;
        }
    }
    return k;
}
__device__ int drop_hyphen_stretches(const cp* s, int n, cp* out) {
    int k = 0, index = 0;
    while (index < n) {
        bool stretched = s[index] == '-' && k > 0 && out[k - 1] != '-' && index + 1 < n && s[index + 1] == out[k - 1];
        if (!stretched) out[k++] = s[index];
        index += 1 + (stretched ? 1 : 0);
    }
    return k;
}

// _Channel.score with _Channel._conditional
__device__ double channel_score(const Channel& c, int order, const cp* keys, int n) {
    cp padded[MAXS + KS_CONTEXT_ACTION_CUDA_ORTHO_ORDER_CAPACITY];     // the host checks the model's order against it
    int length = 0;
    for (int i = 0; i < order - 1; ++i) padded[length++] = ORTHO_BOS;
    for (int i = 0; i < n; ++i) padded[length++] = keys[i];
    padded[length++] = ORTHO_EOS;
    const double* logprob = (const double*)c.logprob_value;
    const double* backoff = (const double*)c.backoff_value;
    Neumaier sum;
    for (int index = order - 1; index < length; ++index) {
        const cp* gram = padded + index - order + 1;
        int gn = order;
        double accumulated = 0.0, value;
        while (true) {
            int found = table_find(c.logprob, gram, gn);
            if (found >= 0) { value = accumulated + logprob[found]; break; }
            if (gn == 1) { value = accumulated + c.uniform; break; }
            int b = table_find(c.backoff, gram, gn - 1);
            accumulated += b >= 0 ? backoff[b] : 0.0;
            gram += 1; gn -= 1;
        }
        sum.add(value);
    }
    return sum.result();
}

__device__ bool split_hyphen(const cp* s, int n, int* head_n) {   // text.split("-") into two non-empty parts
    int hyphens = 0, at = -1;
    for (int i = 0; i < n; ++i) if (s[i] == '-') { ++hyphens; at = i; }
    if (hyphens != 1 || at == 0 || at == n - 1) return false;
    *head_n = at;
    return true;
}
__device__ bool reduplicated(const cp* s, int n) {
    int hn;
    if (!split_hyphen(s, n, &hn)) return false;
    const cp* head = s; const cp* tail = s + hn + 1; int tn = n - hn - 1;
    if (hn >= tn && str_eq(head + hn - tn, tn, tail, tn)) return true;       // head.endswith(tail)
    for (int i = 0; i + tn <= hn; ++i) if (str_eq(head + i, tn, tail, tn)) return true;  // tail in head
    return head[hn - 1] == tail[tn - 1];
}
__device__ bool compound_known(const Model* M, int locale, const cp* s, int n, int min_letters, RowState* st) {
    int hn;
    if (!split_hyphen(s, n, &hn)) return false;
    int tn = n - hn - 1;
    if ((hn > tn ? hn : tn) < min_letters) return false;
    return lm_known(M, locale, s, hn, st) && lm_known(M, locale, s + hn + 1, tn, st);
}
__device__ bool one_key_extra(const Model* M, int locale, const cp* s, int n, RowState* st) {
    if (n < KS_CONTEXT_TYPO_MIN_CHARACTERS || !str_isalpha(M, s, n) || lm_known(M, locale, s, n, st)) return false;
    cp shorter[MAXS];
    for (int index = 0; index < n; ++index) {
        int m = 0;
        for (int i = 0; i < n; ++i) if (i != index) shorter[m++] = s[i];
        if (lm_known(M, locale, shorter, m, st)) return true;
    }
    return false;
}

// OrthoModel._judged's feature sum for one reading; returns the total.
__device__ double ortho_judged(const Model* M, const Ortho* O, const cp* featured, int fn, double source_logprob,
                               double target_logprob, double shape_ratio, int source, int shape, RowState* st) {
    double ratio = target_logprob - source_logprob;
    double features = 0.0;
    if (O->feature_count[source] > 0) {
        int target = 1 - source;
        cp reading[MAXS];
        layout_translate(O, featured, fn, true, reading);       // readings(): keys are the Latin reading
        const cp* src = source == 0 ? featured : reading;
        const cp* tgt = source == 0 ? reading : featured;
        cp lowered[MAXS];
        for (int i = 0; i < fn; ++i) lowered[i] = casefold(M, featured[i]);
        double values[F_COUNT];
        bool unknown = str_isalpha(M, tgt, fn) && !lm_known(M, target, tgt, fn, st);
        bool parts_source = compound_known(M, source, src, fn, (int)O->compound_min[source], st);
        bool parts_target = compound_known(M, target, tgt, fn, (int)O->compound_min[target], st);
        if (O->hyphens && parts_source && parts_target) parts_source = parts_target = false;
        values[F_IDENT] = table_find(O->identifiers, lowered, fn) >= 0;
        values[F_REPEAT] = reduplicated(src, fn) || reduplicated(tgt, fn);
        values[F_PARTS_SOURCE] = parts_source;
        values[F_PARTS_TARGET] = parts_target;
        values[F_DOT_WORD] = source == 1 && fn > 1 && featured[0] == '.' && lm_known(M, 0, featured + 1, fn - 1, st);
        values[F_EXTRA_KEY] = one_key_extra(M, target, tgt, fn, st);
        values[F_SOURCE_WORD] = lm_known(M, source, src, fn, st);
        values[F_NAME_UNKNOWN] = unknown && (shape == SHAPE_INITIAL || shape == SHAPE_INNER);
        values[F_TARGET_UNKNOWN] = unknown;
        values[F_SOURCE_PLAUSIBILITY] = 0.0; values[F_TARGET_PLAUSIBILITY] = 0.0;
        if (O->has_centres) {
            values[F_SOURCE_PLAUSIBILITY] = source_logprob / (double)(fn + 1) - O->centres[source];
            for (int g = 0; g < O->gated_count; ++g) if (values[F_SOURCE_PLAUSIBILITY] >= 0) values[O->gated_code[g]] = 0;
        }
        if (O->has_target_centres) values[F_TARGET_PLAUSIBILITY] = target_logprob / (double)(fn + 1) - O->target_centres[source];
        Neumaier sum;
        for (int f = 0; f < O->feature_count[source]; ++f) sum.add(O->feature_weight[source][f] * values[O->feature_code[source][f]]);
        features = sum.result();
    }
    return ratio + shape_ratio + features;
}

// OrthoModel.score; supported is false for an unscored shape or empty keys.
__device__ bool ortho_score(const Model* M, const Ortho* O, const cp* keys_in, int n, int shape, int source, double* total, RowState* st) {
    if (n == 0 || ((O->unscored_mask >> shape) & 1)) return false;
    cp keys[MAXS];
    for (int i = 0; i < n; ++i) keys[i] = casefold(M, keys_in[i]);
    const Channel& source_channel = O->source_channels[source].present ? O->source_channels[source] : O->channels[source];
    const Channel& target_channel = O->channels[1 - source];
    double source_acronym = O->acronym_shape[source][shape] - O->prose_shape[source][shape];
    double shape_ratio = -source_acronym;
    cp typed[MAXS], once[MAXS], dropped[MAXS], cut[MAXS];
    int tn = O->collapse > 0 ? collapse_runs(keys, n, (int)O->collapse, typed) : n;
    if (O->collapse <= 0) for (int i = 0; i < n; ++i) typed[i] = keys[i];
    int readings = 1, on = 0;
    if (O->stretch > 0) {
        const cp* base = keys; int bn = n;
        if (O->hyphens) { bn = drop_hyphen_stretches(keys, n, dropped); base = dropped; }
        on = read_once(base, bn, (int)O->stretch, cut);
        if (O->collapse > 0) on = collapse_runs(cut, on, (int)O->collapse, once);
        else for (int i = 0; i < on; ++i) once[i] = cut[i];
        if (!str_eq(once, on, typed, tn)) readings = 2;
    }
    double best = 0.0;
    for (int r = 0; r < readings; ++r) {
        const cp* letters = r == 0 ? typed : once;
        int ln = r == 0 ? tn : on;
        const cp* featured = O->stretch > 0 ? letters : keys;
        int fn = O->stretch > 0 ? ln : n;
        double value = ortho_judged(M, O, featured, fn, channel_score(source_channel, (int)O->order, letters, ln),
                                    channel_score(target_channel, (int)O->order, letters, ln), shape_ratio, source, shape, st);
        if (r == 0 || value < best) best = value;
    }
    *total = best;
    return true;
}

// ortho_model.shape_of
__device__ int shape_of(const Model* M, const cp* token, int n, bool first_in_field) {
    int letters = 0; bool all_upper = true;
    for (int i = 0; i < n; ++i) {
        unsigned p = props(M, token[i]);
        if (!(p & P_ALPHA)) continue;
        ++letters;
        if (!(p & P_UPPER)) all_upper = false;
    }
    if (letters >= KS_ACRONYM_MIN_LETTERS && all_upper) return SHAPE_UPPER;
    if (n > 0 && (props(M, token[0]) & P_UPPER)) return first_in_field ? SHAPE_INITIAL : SHAPE_INNER;
    return SHAPE_LOWER;
}

// context_model.one_typo_from_word over the frequency table.
__device__ bool one_typo(const Model* M, const EvidenceTables* E, int locale, const cp* text, int n) {
    cp word[MAXS];
    int k = lm_normalize(M, text, n, word);
    if (k < KS_CONTEXT_TYPO_MIN_CHARACTERS || k != n) return false;
    for (int i = 0; i < n; ++i) if (word[i] != casefold(M, text[i])) return false;
    if (!str_isalpha(M, word, k) || table_find(M->freq[locale], word, k) >= 0) return false;
    const cp* letters = (const cp*)E->alphabet[locale];
    int ln = (int)E->alphabet_n[locale];
    cp probe[MAXS + 1];
    for (int index = 0; index < k; ++index) {
        cp current = word[index];
        int m = 0;
        for (int i = 0; i < k; ++i) if (i != index) probe[m++] = word[i];
        if (table_find(M->freq[locale], probe, m) >= 0) return true;
        if (index + 1 < k) {
            for (int i = 0; i < k; ++i) probe[i] = word[i];
            probe[index] = word[index + 1]; probe[index + 1] = current;
            if (table_find(M->freq[locale], probe, k) >= 0) return true;
        }
        for (int i = 0; i < k; ++i) probe[i] = word[i];
        for (int l = 0; l < ln; ++l) {
            if (letters[l] == current) continue;
            probe[index] = letters[l];
            if (table_find(M->freq[locale], probe, k) >= 0) return true;
        }
    }
    for (int index = 0; index <= k; ++index)
        for (int l = 0; l < ln; ++l) {
            int m = 0;
            for (int i = 0; i < index; ++i) probe[m++] = word[i];
            probe[m++] = letters[l];
            for (int i = index; i < k; ++i) probe[m++] = word[i];
            if (table_find(M->freq[locale], probe, m) >= 0) return true;
        }
    return false;
}

struct Evidence {            // the ContextEvidence fields the features read, of one ActionRow
    WordScore source, target;
    bool baseline_convert, has_prediction; double calibrated, model_threshold;
    bool source_identifier, target_identifier, source_typo, target_typo, source_opening, target_opening;
    bool has_ortho; double ortho_score, ortho_threshold;
};

// context_policy.evidence_for_decision with the trainer's arguments.
__device__ void evidence_for_decision(const Model* M, const Decide* Dc, const EvidenceTables* E, const Row& r, bool spelling,
                                      bool identifiers_dropped, const Decision& d, const cp* alternative, int an,
                                      Evidence* e, RowState* st) {
    int sg = r.group, tg = 1 - r.group;
    e->source = lm_score(M, sg, spelling, r.original, r.original_n, st);
    e->target = lm_score(M, tg, spelling, alternative, an, st);
    cp lowered[MAXS];
    if (identifiers_dropped) { e->source_identifier = e->target_identifier = false; }
    else {
        for (int i = 0; i < r.original_n; ++i) lowered[i] = casefold(M, r.original[i]);
        e->source_identifier = table_find(E->ortho.identifiers, lowered, r.original_n) >= 0;
        for (int i = 0; i < an; ++i) lowered[i] = casefold(M, alternative[i]);
        e->target_identifier = table_find(E->ortho.identifiers, lowered, an) >= 0;
    }
    const cp* keys = sg == 0 ? r.original : alternative;
    int kn = sg == 0 ? r.original_n : an;
    bool first = !str_has_nonspace(M, r.before, r.before_n);
    e->has_ortho = ortho_score(M, &E->ortho, keys, kn, shape_of(M, r.original, r.original_n, first), sg, &e->ortho_score, st);
    e->ortho_threshold = e->has_ortho ? E->ortho.thresholds[sg] : 0.0;
    e->baseline_convert = d.should_convert; e->has_prediction = d.has_prediction;
    e->calibrated = d.calibrated; e->model_threshold = d.threshold;
    e->source_typo = one_typo(M, E, sg, r.original, r.original_n);
    e->target_typo = one_typo(M, E, tg, alternative, an);
    cp normalized[MAXS];
    int k = lm_normalize(M, r.original, r.original_n, normalized);
    e->source_opening = table_find(E->opening[sg], normalized, k) >= 0;
    k = lm_normalize(M, alternative, an, normalized);
    e->target_opening = table_find(E->opening[tg], normalized, k) >= 0;
}
