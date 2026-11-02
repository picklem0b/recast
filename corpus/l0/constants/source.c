static const int LIMIT = 10;
static const int STEP = 3;

int capped(int n) {
    if (n > LIMIT) {
        return LIMIT;
    }
    return n;
}

int stepped(int n) {
    return n * STEP + LIMIT;
}

int loop_to_limit(void) {
    int total = 0;
    for (int i = 0; i < LIMIT; i++) {
        total += i;
    }
    return total;
}

/* Literals whose C type Rust cannot infer from the surrounding expression: a
   value above `i32`, and the most negative `int`, whose literal is a positive
   `long` that is then negated. */
long wide(void) {
    return 3000000000L;
}

int below_int_min(void) {
    return -2147483648;
}

/* An unsigned value whose high bit is set: the exported literal is negative
   before it is spelled at its own width. */
unsigned int high_bit(void) {
    return 0x80000000u;
}

/* Radix and character literals keep their values. */
int radix(void) {
    return 0x1F + 010 + 5;
}

int chars(void) {
    return 'A' + '\n' + '0';
}

/* Unsigned literals keep their value: sign-extending them would turn
   4294967295 into -1. */
unsigned int all_bits(void) {
    return 0xFFFFFFFFu;
}

unsigned long max_unsigned(void) {
    return 18446744073709551615ul;
}
