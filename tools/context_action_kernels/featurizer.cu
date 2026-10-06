// Production kernels of the CUDA featurizer.
#include "features.cuh"

#define COLUMN_PARAMS const cp* original_pool, const i64* original_offsets, const int* original_lengths, const cp* alternative_pool, const i64* alternative_offsets, const int* alternative_lengths, const cp* before_pool, const i64* before_offsets, const int* before_lengths, const cp* after_pool, const i64* after_offsets, const int* after_lengths, const cp* application_pool, const i64* application_offsets, const int* application_lengths, const cp* role_pool, const i64* role_offsets, const int* role_lengths, const cp* trigger_pool, const i64* trigger_offsets, const int* trigger_lengths, const cp* tail_pool, const i64* tail_offsets, const int* tail_lengths, const cp* boundary_pool, const i64* boundary_offsets, const int* boundary_lengths
#define COLUMN_AT(i) FeatureRow{Text{original_pool + original_offsets[(i)], original_lengths[(i)]}, Text{alternative_pool + alternative_offsets[(i)], alternative_lengths[(i)]}, Text{before_pool + before_offsets[(i)], before_lengths[(i)]}, Text{after_pool + after_offsets[(i)], after_lengths[(i)]}, Text{application_pool + application_offsets[(i)], application_lengths[(i)]}, Text{role_pool + role_offsets[(i)], role_lengths[(i)]}, Text{trigger_pool + trigger_offsets[(i)], trigger_lengths[(i)]}, Text{tail_pool + tail_offsets[(i)], tail_lengths[(i)]}, Text{boundary_pool + boundary_offsets[(i)], boundary_lengths[(i)]}}

// ActionRow -> Record (decision and evidence_for_decision). The probability field holds the calibrated
// logit until the host applies stable_sigmoid. states: ROW_* (CONTEXT_ACTION_CUDA_ROW_STATES): done, waits
// for Hunspell answers, takes the CPU path.
extern "C" __global__ void row_records(const unsigned char* model, const unsigned char* decide, const unsigned char* evidence,
        const unsigned char* misses, COLUMN_PARAMS, const int* groups, const int* trigger_indices, const int* origins,
        const unsigned char* dropped, const int* rows, int count, int spelling, Record* records, int* states) {
    int k = blockIdx.x * blockDim.x + threadIdx.x;
    if (k >= count) return;
    int i = rows[k];
    if (states[k] != ROW_ASKING) return;
    const Model* M = (const Model*)model; const Decide* Dc = (const Decide*)decide; const EvidenceTables* E = (const EvidenceTables*)evidence;
    RowState st = {(const Misses*)misses, false, false};
    FeatureRow fr = COLUMN_AT(i);
    Row r = Row{fr.original.s, fr.original.n, groups[i], fr.before.s, fr.before.n, fr.after.s, fr.after.n,
                fr.application.s, fr.application.n, fr.trigger.s, fr.trigger.n, trigger_indices[i], origins[i] == ORIGIN_PLANNED};
    Previous prev; previous_context(M, Dc, r, &prev, &st);
    if (st.fallback) { states[k] = ROW_CPU; return; }
    Decision d = automatic_word_decision(M, Dc, r, spelling, fr.alternative.s, fr.alternative.n, &prev, &st);
    Evidence e; evidence_for_decision(M, Dc, E, r, spelling, dropped[i], d, fr.alternative.s, fr.alternative.n, &e, &st);
    if (st.fallback) { states[k] = ROW_CPU; return; }
    if (st.incomplete) return;
    Record& o = records[k];
    const WordScore* w[KS_LAYOUT_GROUP_COUNT] = {&e.source, &e.target};
    double* sides[KS_LAYOUT_GROUP_COUNT] = {o.source, o.target};
    for (int s = 0; s < KS_LAYOUT_GROUP_COUNT; ++s) {
        sides[s][SCORE_VALUE] = w[s]->value; sides[s][SCORE_GRAM_RATIO] = w[s]->gram_ratio; sides[s][SCORE_NGRAM_SCORE] = w[s]->ngram_score;
        sides[s][SCORE_INVALID_RATIO] = w[s]->invalid_ratio; sides[s][SCORE_RAW_NGRAM_SCORE] = w[s]->raw_ngram_score;
    }
    o.source_frequency = e.source.frequency; o.target_frequency = e.target.frequency;
    o.source_flags = (e.source.exact ? SF_EXACT : 0) | (e.source.spell_known ? SF_SPELL_KNOWN : 0) | SF_PRESENT;
    o.target_flags = (e.target.exact ? SF_EXACT : 0) | (e.target.spell_known ? SF_SPELL_KNOWN : 0) | SF_PRESENT;
    o.score_delta = e.target.value - e.source.value;
    i64 flags = 0;
    if (e.has_prediction) { o.probability = e.calibrated; o.threshold = e.model_threshold; flags |= RF_PROBABILITY | RF_THRESHOLD; }
    if (e.has_ortho) { o.ortho_score = e.ortho_score; o.ortho_threshold = e.ortho_threshold; flags |= RF_ORTHO_SCORE | RF_ORTHO_THRESHOLD; }
    if (e.baseline_convert) flags |= RF_BASELINE;
    if (e.source.known) flags |= RF_SOURCE_KNOWN;
    if (e.target.known) flags |= RF_TARGET_KNOWN;
    if (e.source_identifier) flags |= RF_SOURCE_IDENTIFIER;
    if (e.target_identifier) flags |= RF_TARGET_IDENTIFIER;
    if (e.source_opening) flags |= RF_SOURCE_OPENING;
    if (e.target_opening) flags |= RF_TARGET_OPENING;
    o.flags = flags; o.source_group = r.group;
    // ContextEvidence.__post_init__: none/field follow whether the field has text after the word
    o.after_origin = origins[i] == ORIGIN_PLANNED ? ORIGIN_PLANNED : (fr.after.n > 0 ? ORIGIN_FIELD : ORIGIN_NONE);
    states[k] = ROW_DONE;
}

// Record + strings -> features, written at an atomically reserved place; in name mode (wanted != null)
// only the wanted names are written.
extern "C" __global__ void row_features(const unsigned char* model, const unsigned char* ftables, COLUMN_PARAMS,
        const int* rows, const Record* records, const int* states, int count, const Wanted* wanted,
        unsigned long long* cursor, i64 capacity, u64* out_h1, u64* out_h2, double* out_v, i64* row_start, int* row_n, int* row_state) {
    int k = blockIdx.x * blockDim.x + threadIdx.x;
    if (k >= count) return;
    if (states[k] != ROW_DONE) { if (wanted == nullptr) { row_state[k] = ROW_CPU; row_n[k] = 0; } return; }
    const Model* M = (const Model*)model; const FeatureTables* Ft = (const FeatureTables*)ftables;
    int i = rows[k];
    FeatureRow fr = COLUMN_AT(i);
    FeatureMap f; f.clear(wanted);
    bool ok = extract_action_features(f, M, Ft, fr, records[k]);
    if (wanted != nullptr) return;
    if (!ok) { row_state[k] = ROW_CPU; row_n[k] = 0; return; }
    int n = 0;
    for (int e = 0; e < f.n; ++e) n += f.v[e] != 0.0;
    unsigned long long start = atomicAdd(cursor, (unsigned long long)n);
    if ((i64)(start + n) > capacity) { row_state[k] = ROW_RETRY; row_n[k] = 0; return; }   // no room: retry in a later pass
    int w = 0;
    for (int e = 0; e < f.n; ++e) if (f.v[e] != 0.0) {
        out_h1[start + w] = f.h1[e]; out_h2[start + w] = f.h2[e]; out_v[start + w] = f.v[e]; ++w;
    }
    row_start[k] = (i64)start; row_n[k] = n; row_state[k] = ROW_DONE;
}
