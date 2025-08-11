declare module '*.module.css' {
    const classes: { readonly [key: string]: string }
    export default classes
}

export type { I1 } from 'mystery-module'
export { numberFunc } from 'unknown-library'
export interface I2 {}

export interface CreatePreloadedQueryResult<D extends object, V extends I1> {
    queryLoader: (variables: V) => Promise<Record<string, I2 | undefined>>
    usePreloadedQueryData: () => ReturnType<typeof numberFunc>
}
