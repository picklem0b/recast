#include <stddef.h>

int sum(const int *a, size_t n) {
    int s = 0;
    for (size_t i = 0; i < n; i++) {
        s += a[i];
    }
    return s;
}
