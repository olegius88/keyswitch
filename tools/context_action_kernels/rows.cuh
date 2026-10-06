// Row access for the kernels: five UTF-32 string columns and three integer columns.
#pragma once
#include "decide.cuh"

#define ROW_PARAMS const cp* original_pool, const i64* original_offsets, const int* original_lengths, \
    const cp* before_pool, const i64* before_offsets, const int* before_lengths, \
    const cp* after_pool, const i64* after_offsets, const int* after_lengths, \
    const cp* application_pool, const i64* application_offsets, const int* application_lengths, \
    const cp* trigger_pool, const i64* trigger_offsets, const int* trigger_lengths, \
    const int* groups, const int* trigger_indices, const int* planned_flags

#define ROW_AT(i) Row{original_pool + original_offsets[i], original_lengths[i], groups[i], \
    before_pool + before_offsets[i], before_lengths[i], after_pool + after_offsets[i], after_lengths[i], \
    application_pool + application_offsets[i], application_lengths[i], \
    trigger_pool + trigger_offsets[i], trigger_lengths[i], trigger_indices[i], planned_flags[i]}
