// both 'jquery' and '@types/jquery' packages exist
// 'declare module' should be ignored
import 'jquery';

// 'lodash' package exists
// 'declare module' should be ignored
import 'lodash';

// '@types/testing-library__jest-dom' package exists
// 'declare module' should be ignored
import '@testing-library/jest-dom';

// has multiple 'declare module' definitions and no packages, all should be included
import 'lib-a';
import 'lib-b';
