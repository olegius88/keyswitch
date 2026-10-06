/* Training-only: FeatureMass.add of the context-v3 trainer over packed rows, in row order. Every
   feature of a row adds the row's sample weight to its name's compensated sum; 1 when a sum stops
   being finite, as the class refuses. Built with the flags of tools/context_optimizer.c. */
#include <stdint.h>
#include <math.h>

int context_action_feature_mass(const uint32_t *ids, const uint64_t *offsets, const double *weights,
                                uint64_t rows, double *values, double *errors) {
    for (uint64_t row = 0; row < rows; ++row) {
        for (uint64_t j = offsets[row]; j < offsets[row + 1]; ++j) {
            double previous = values[ids[j]];
            double increment = weights[row] - errors[ids[j]];
            double total = previous + increment;
            if (!isfinite(total)) return 1;
            errors[ids[j]] = (total - previous) - increment;
            values[ids[j]] = total;
        }
    }
    return 0;
}
