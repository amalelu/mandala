# Code Conventions

## First of all
The following conventions belong to the *repository* first and foremost, not to developers.
Code does evolve, and it is unreasonable to expect that it will always be completely up to par 
with the conventions. However, we can always be working to keep pushing the codebase's evolution 
towards complete conformity to the conventions. Conventions may change, and in fact we would
much rather have non-conventional contributions, if the code is good enough, than no contributions.

So it is in this spirit that we should interpret and enforce conventions. If someone who is busy but
contributes good code did not follow conventions, we accept the merge request and then someone else
can contribute refactors such that it follows convention.

## Breaking Conventions
Sometimes it may make sense to deliberately break the conventions, in such cases one should document ones
reasoning for doing so at the level of scope that one intends to do it. If code reviewers agree that the
given case makes sense, this particular code should not be refactored to follow convention, obviously.

## Fundamental Conventions
We follow the fundamental code styles and conventions of the languages we use as a basis.

## Design and code structure
### §1 Keep It Simple, Stupid
Follow the *K I S S* principle. Keep things as simple as possible, but by all means, do not try to push
a complex reality into a too simple design. It is a fine art to strike a balance between simplicity, scalability, 
sustainability, etc. Since it is an art, there is no point trying to define a right way, that is your responsibility
as the artist.

### §2 Readability, maintainability, art
Coding is an art, but also it consists of numerous techniques that are well-defined. We want our code to be
as easy to understand and maintain as possible, but we also want it to be beautiful and inspiring.
Art thrives under limitations but not under strict recipes. We want you to be an artist, but first and foremost
we want the code to be easy to understand. If it is beautiful, humorous, poetic, etc. then even better.
