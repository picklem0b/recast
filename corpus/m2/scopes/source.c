#include <stddef.h>

/*
 * The scopes the frontend must keep accepting: a name reused by two sibling
 * blocks, an index reused by two sibling loops, and integer compound
 * assignments. None of these is shadowing, so none is refused, and each must
 * keep the behavior of the C below.
 */

/* Two sibling blocks may each declare `step`: neither is in the other's
 * scope, and each definition precedes its only use. */
int sibling_total(int seed) {
    int total = seed;
    {
        int step = 1;
        total += step;
    }
    {
        int step = 2;
        total += step;
    }
    return total;
}

/* Two loops may each own the index `i`; the second one starts from zero
 * because the first one's declaration ended with its loop. */
int loop_totals(int n) {
    int total = 0;
    for (int i = 0; i < n; i++) {
        total += i;
    }
    for (int i = 0; i < 2; i++) {
        total -= i;
    }
    return total;
}

/* Integer compound assignments compute in the target's type and wrap. */
unsigned int mix(unsigned int u, int s, unsigned char c) {
    u *= 3u;
    u /= 2u;
    s += 7;
    s -= 3;
    c *= 2;
    return u + (unsigned int)s + (unsigned int)c;
}

int scale(int x, int k) {
    x *= k;
    x += 1;
    x /= 2;
    return x;
}
