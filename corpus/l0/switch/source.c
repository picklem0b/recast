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

/* Unsigned case labels keep their value too. */
int classify_unsigned(unsigned int key) {
    switch (key) {
        case 4000000000u:
            return 4;
        case 2147483648u:
            return 2;
        default:
            return 0;
    }
}

/* Arms with several statements: clang leaves the ones after the first as
   siblings of the label, and dropping them changed the result silently. */
int weighted(int key, int value) {
    int total = 0;
    switch (key) {
        case 0:
            total += value;
            total *= 3;
            break;
        case 1:
            total -= value;
            if (total < 0) {
                total += 10;
            }
            break;
        default:
            total = value + 1;
            total ^= 5;
    }
    return total;
}

/* A trailing compound statement in the arm, and a `default` that is last with
   several statements and no `break`. */
int scaled(int key, int value) {
    int total = 1;
    switch (key) {
        case 2: {
            int scaled = value * 2;
            total = scaled + 1;
            break;
        }
        default:
            total = value;
            total -= 7;
    }
    return total;
}

/* A `continue` in an arm leaves the switch for the loop; it is not
   fallthrough into the next case. */
int skip_threes(int limit) {
    int total = 0;
    for (int i = 0; i < limit; i++) {
        switch (i % 3) {
            case 0:
                continue;
            case 1:
                total += i;
                break;
            default:
                total += 10;
        }
        total += 100;
    }
    return total;
}
