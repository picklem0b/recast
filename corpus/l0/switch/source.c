#include <stddef.h>

/* A `default` arm that is not last must still lose to a matching case. */
int classify(int c) {
    switch (c) {
        default:
            return 99;
        case 0:
            return 10;
        case 1:
            return 20;
    }
}

/* A loop body that both continues and breaks: neither may leave the other
   unlabeled, and the step must still run on the continue path. */
int scan(const int *a, size_t n, int limit) {
    int total = 0;
    for (size_t i = 0; i < n; i++) {
        if (a[i] < 0) {
            continue;
        }
        if (total > limit) {
            break;
        }
        total += a[i];
    }
    return total;
}

/* The same two statements inside a `do-while`, where the condition runs after
   the body. */
int count_positive(const int *a, size_t n) {
    int count = 0;
    size_t i = 0;
    do {
        if (a[i] <= 0) {
            i += 1;
            continue;
        }
        if (count > 3) {
            break;
        }
        count += 1;
        i += 1;
    } while (i < n);
    return count;
}

/* A switch with no default arm at all. */
int low_nibble(int value) {
    switch (value & 0xF) {
        case 0:
            return 0;
        case 15:
            return 15;
    }
    return -1;
}
