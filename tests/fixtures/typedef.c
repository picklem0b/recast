typedef struct {
    int a;
    int b;
} Pair;

Pair make(int a, int b) {
    Pair p = {a, b};
    return p;
}

int sum(Pair p) {
    return p.a + p.b;
}
