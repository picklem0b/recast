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
