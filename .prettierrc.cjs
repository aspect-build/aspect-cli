/* eslint-env node */

module.exports = {
    trailingComma: 'es5',
    tabWidth: 4,
    semi: true,
    singleQuote: true,
    overrides: [
        {
            files: ['**/*.yaml', '**/*.yml', '**/*.json'],
            options: {
                tabWidth: 2,
            },
        },
    ],
};
