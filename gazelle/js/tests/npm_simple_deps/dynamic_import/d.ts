import('jquery').then(($) =>
    import('@aspect-test/c').then((c) => {
        console.log($(c));
    })
);
