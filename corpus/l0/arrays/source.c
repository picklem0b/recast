#include <stddef.h>

/* A local array: full, partial and assigned initialization, and mutation. */
int full(void) {
    int xs[3] = {1, 2, 3};
    return xs[0] + xs[1] + xs[2];
}

int partial(void) {
    int xs[3] = {7};
    int total = 0;
    for (int i = 0; i < 3; i++) {
        total = total * 2 + xs[i];
    }
    return total;
}

int assigned(void) {
    int xs[3];
    xs[0] = 5;
    xs[1] = 6;
    xs[2] = 7;
    return xs[0] + xs[1] + xs[2];
}

int mutated(void) {
    int xs[3] = {1, 2, 3};
    xs[1] = 9;
    return xs[0] * xs[1] * xs[2];
}

/* An array field of a record by value, read and written through the field. */
struct window {
    int values[3];
    int weight;
};

int total(struct window w) {
    int sum = 0;
    for (size_t i = 0; i < 3; i++) {
        sum += w.values[i];
    }
    return sum + w.weight;
}

int bump(struct window w) {
    w.values[1] += 10;
    return w.values[1] + w.weight;
}

/* An array of arrays: element `i` is itself an array in place. */
int grid(void) {
    int g[2][3] = {{1, 2, 3}, {4, 5, 6}};
    return g[1][2] + g[0][0];
}
