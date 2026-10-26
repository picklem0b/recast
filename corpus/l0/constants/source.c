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
