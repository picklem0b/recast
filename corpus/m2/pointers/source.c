#include <stddef.h>

/* An output parameter: the callee writes through the pointer. */
void bump(int *value) {
    *value += 1;
}

int add_one(int n) {
    bump(&n);
    return n;
}

/* A read-only pointer parameter. */
int read(const int *value) {
    return *value;
}

void add_to(int *value, int delta) {
    *value += delta;
}

unsigned int set_high(unsigned int *value) {
    *value |= 0x100u;
    return *value;
}

/* Two pointers in one call. */
int sum_with(int *a, int *b) {
    return *a + *b;
}

/* A pointer local, passed twice: each call copies the pointer in C. */
int twice_through(int n) {
    int *p = &n;
    bump(p);
    bump(p);
    return *p;
}

/* The same place read through a pointer and a value in one expression. */
int total(int n) {
    int *p = &n;
    return *p + n;
}
