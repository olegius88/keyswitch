/* Training-only sparse AdaGrad softmax; no native dependency in KeySwitch. */
#include <stdint.h>
#include <math.h>

static void probabilities(const uint64_t *offsets, const uint32_t *indices,
                          const double *values, uint64_t row,
                          const double *weights, double *p) {
    for (int k = 0; k < 4; ++k) p[k] = 0.0;
    for (uint64_t j = offsets[row]; j < offsets[row + 1]; ++j)
        for (int k = 0; k < 4; ++k)
            p[k] += weights[(uint64_t)indices[j] * 4 + k] * values[j];
    double maximum = p[0], total = 0.0;
    for (int k = 1; k < 4; ++k) if (p[k] > maximum) maximum = p[k];
    for (int k = 0; k < 4; ++k) { p[k] = exp(p[k] - maximum); total += p[k]; }
    for (int k = 0; k < 4; ++k) p[k] /= total;
}

void context_epoch(const uint64_t *offsets, const uint32_t *indices,
                   const double *values, const uint8_t *labels,
                   const double *importance, uint64_t rows,
                   double *weights, double *accumulators, double rate) {
    double p[4];
    for (uint64_t row = 0; row < rows; ++row) {
        probabilities(offsets, indices, values, row, weights, p);
        for (uint64_t j = offsets[row]; j < offsets[row + 1]; ++j) {
            for (int k = 0; k < 4; ++k) {
                uint64_t index = (uint64_t)indices[j] * 4 + k;
                double g = importance[row] * (p[k] - (k == labels[row])) * values[j];
                accumulators[index] += g * g;
                weights[index] -= rate * g / sqrt(accumulators[index]);
            }
        }
    }
}

void context_predict(const uint64_t *offsets, const uint32_t *indices,
                     const double *values, uint64_t rows,
                     const double *weights, double *output) {
    for (uint64_t row = 0; row < rows; ++row)
        probabilities(offsets, indices, values, row, weights, output + row * 4);
}
