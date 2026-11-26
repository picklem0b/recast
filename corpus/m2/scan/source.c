#include <stddef.h>

/*
 * A scanning and accounting case: the kinds of loops, byte tests and
 * accumulator updates that a parser or a checksum routine is made of. Every
 * function stays inside the accepted M2 subset: slices for buffers, one
 * pointer-to-object for an output parameter, and a record behind a pointer
 * for the accumulator.
 */

enum kind { KIND_ZERO = 0, KIND_ONE = 1, KIND_MANY = 2 };

struct stats {
    int min;
    int max;
    int total;
};

/* A byte test with two ranges and a default. */
int hex_digit(int c) {
    if (c >= '0' && c <= '9') {
        return c - '0';
    }
    if (c >= 'a' && c <= 'f') {
        return c - 'a' + 10;
    }
    if (c >= 'A' && c <= 'F') {
        return c - 'A' + 10;
    }
    return -1;
}

/* A rolling checksum: unsigned arithmetic wraps, so the Rust must wrap too. */
unsigned int checksum(const char *s, size_t n) {
    unsigned int hash = 2166136261u;
    for (size_t i = 0; i < n; i++) {
        hash = (hash ^ (unsigned int)(unsigned char)s[i]) * 16777619u;
    }
    return hash;
}

/* Count the digits in a buffer: a loop with a counter and a byte test. */
int count_digit(const char *s, size_t n) {
    int digits = 0;
    for (size_t i = 0; i < n; i++) {
        if (s[i] >= '0' && s[i] <= '9') {
            digits++;
        }
    }
    return digits;
}

/* Sum only the valid hexadecimal digits, stopping at the first bad one. */
int hex_sum(const char *s, size_t n) {
    int total = 0;
    for (size_t i = 0; i < n; i++) {
        int digit = hex_digit(s[i]);
        if (digit < 0) {
            break;
        }
        total += digit;
    }
    return total;
}

/* An output parameter with a boundary: the callee writes through it. */
void clamp_value(int *out, int v, int limit) {
    if (v > limit) {
        *out = limit;
    } else if (v < -limit) {
        *out = -limit;
    } else {
        *out = v;
    }
}

/* A record behind a pointer: the callee updates all three fields. */
void accumulate(struct stats *s, int v) {
    if (v < s->min) {
        s->min = v;
    }
    if (v > s->max) {
        s->max = v;
    }
    s->total += v;
}

/* A const record behind a pointer: read the fields, do not write them. */
int spread_of(const struct stats *s) {
    return s->max - s->min;
}

/* A mutable slice and a const slice: the output holds the doubled input. */
void double_into(int *out, const int *src, size_t n) {
    for (size_t i = 0; i < n; i++) {
        out[i] = src[i] * 2;
    }
}

/* A switch over an enum with two cases and a default. */
int classify(int v) {
    switch (v) {
    case 0:
        return KIND_ZERO;
    case 1:
        return KIND_ONE;
    default:
        return KIND_MANY;
    }
}

/* A fixed table and a nested loop: the inner total depends on the outer index. */
int triangular(void) {
    int table[4] = {1, 2, 3, 4};
    int total = 0;
    for (int i = 0; i < 4; i++) {
        for (int j = 0; j <= i; j++) {
            total += table[j];
        }
    }
    return total;
}

/* Build the accumulator from scalars, then read it back. */
int stats_of(int lo, int hi, int step) {
    struct stats s;
    s.min = lo;
    s.max = lo;
    s.total = 0;
    accumulate(&s, lo);
    accumulate(&s, hi);
    accumulate(&s, step);
    return s.total * 1000 + spread_of(&s);
}
