#include <stddef.h>

long sum(const int *a, size_t n) {
    long total = 0;
    for (size_t i = 0; i < n; i++) {
        total += a[i];
    }
    return total;
}

int first_negative(const int *a, size_t n) {
    for (size_t i = 0; i < n; i++) {
        if (a[i] < 0) {
            return (int)i;
        }
    }
    return -1;
}

int count_char(const char *s, size_t n) {
    int count = 0;
    for (size_t i = 0; i < n; i++) {
        if (s[i] == 'a') {
            count++;
        }
    }
    return count;
}

int dot(const int *a, const int *b, size_t n) {
    int total = 0;
    for (size_t i = 0; i < n; i++) {
        total += a[i] * b[i];
    }
    return total;
}

int first(const int *a, size_t n) {
    return a[0];
}

int first_or(const int *a, size_t n) {
    if (n == 0) {
        return -1;
    }
    return a[0];
}

long head(const int *a, size_t n) {
    long total = 0;
    for (size_t i = 0; i < n; i++) {
        total += a[i];
    }
    return total;
}

/* A call may pass fewer elements than the array holds. */
long short_call(void) {
    int a[4] = {10, 20, 30, 40};
    return head(a, 2);
}

/* And it may start in the middle of one. */
long offset_call(void) {
    int a[4] = {10, 20, 30, 40};
    return head(&a[1], 2);
}
