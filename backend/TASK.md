Create a Nest.js backend application with the following functionality:
- Audio recording rating service for use in a machine learning model
Stack:
- Nest.js
- TailwindCSS 4
- Modern JS/TS
- PostgreSQL
- Modern technologies and frameworks
Database access details:
- Address: localhost:5432
- DB: libvoice
- User: libvoice
- Password: 1111
- Be sure to implement migrations. Don't forget about indexes
Tables:
- recordings - the table storing the recording path relative to the base storage path, speaker ID (if available), tags (if available)
- votes - the table storing human-generated ratings associated with a specific recording
- It should be possible to expand these tables later. New tables can be added if necessary for the implementation.

The frontend should include a web application that immediately prompts the user to rate a recording. After the rating is complete, the application moves on to the next step. The frontend should not include administrative operations, only the user interface. Administrative operations should be implemented in console commands.

The starting data source will be the voxceleb2 database located at /media/data/experiment/voxeleb2/. The console command should contain instructions for loading data into the database from the voxceleb2 metadata.

Voice evaluation:

We will label the data for the AI ​​model. The scales the model will use are:
- Femininity-Masculinity (0-1)
- Naturalness (0-1)
- Attractiveness (0-1)
We need to obtain a continuous scale. To achieve this, the service should present the user with two audio recordings from different speakers and ask them: which of these recordings sounds more feminine/masculine, which is more natural, and which is more attractive.

In the first phase of training, we only allow the user to choose speakers of the same gender. The priority is to compare different speakers. The interface should also include a button to reject a recording of one speaker, for example, if it's too noisy or of poor quality for evaluation. In this case, another recording of the same speaker is provided.

In the second phase of training, we will examine controversial cases across genders, for example, comparing the 20% of the lowest female voices with the 20% of the highest male voices (based on the femininity-masculinity assessment).

Develop a logic that allows for a small but sufficient number of comparisons. Prioritize analyzing complex/controversial cases. The data obtained during the work should be suitable for Bradley-Terry/Thurstone scaling.
