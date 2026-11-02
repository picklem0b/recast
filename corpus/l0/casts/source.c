#include <stddef.h>

/* A cast of a constant is a constant: C truncates to the target width
   before the value is used, so the generated Rust must say 44, not 300. */
int narrow_unsigned_char(void) {
    return (unsigned char)300;
}

int narrow_char(void) {
    return (char)300;
}

int narrow_short(void) {
    return (short)70000;
}

unsigned int negated(void) {
    return (unsigned int)-1;
}

/* Widening and sign changes of a runtime value stay casts. */
long widened(int n) {
    return (long)n;
}

unsigned int unsigned_of_int(int n) {
    return (unsigned int)n;
}

/* A cast on the left of a comparison keeps its parentheses: Rust reads
   `n as u32 < 3` as generic arguments and rejects it. */
int below(int n) {
    return (unsigned int)n < 3;
}
