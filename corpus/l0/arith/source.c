int add(int a, int b) {
    return a + b;
}

int sub(int a, int b) {
    return a - b;
}

int mul(int a, int b) {
    return a * b;
}

int div_mod(int a, int b) {
    return a / b + a % b;
}

int shift_mix(unsigned int a, unsigned int b) {
    return (int)((a << 3) + (b >> 2));
}

unsigned int wrap_add(unsigned int a, unsigned int b) {
    return a + b;
}

unsigned int wrap_sub(unsigned int a, unsigned int b) {
    return a - b;
}

unsigned int wrap_mul(unsigned int a, unsigned int b) {
    return a * b;
}

int negate(int a) {
    return -a;
}

int bit_mix(unsigned int a) {
    return (int)(~a ^ (a >> 1));
}

int min_int(int a, int b) {
    if (a < b) {
        return a;
    }
    return b;
}

int shift_assign(int a, int n) {
    a <<= 1;
    a >>= n;
    a <<= 2;
    return a;
}

unsigned int shift_assign_unsigned(unsigned int a, int n) {
    a <<= 3;
    a >>= 1;
    return a;
}
